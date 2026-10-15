use super::*;
use rustix::fs::AtFlags;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use rustix::fs::Stat;
use rustix::fs::fstat;
use rustix::fs::open;
use rustix::fs::openat;
use rustix::fs::renameat;
use rustix::fs::statat;
use rustix::fs::unlinkat;
use rustix::process::getuid;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::Component;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug)]
pub(super) struct SelectedFile {
    path: PathBuf,
    parent: OwnedFd,
    name: OsString,
    operations: Mutex<()>,
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsafe or replaced auth-file path",
    )
}
fn same(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}
fn private_file(s: &Stat) -> io::Result<()> {
    if s.st_uid != getuid().as_raw()
        || s.st_mode & 0o170000 != 0o100000
        || s.st_mode & 0o7777 != 0o600
        || s.st_nlink != 1
    {
        return Err(denied());
    }
    Ok(())
}
fn parent(path: &Path) -> io::Result<(OwnedFd, OsString)> {
    let components = path.components().collect::<Vec<_>>();
    if components.len() < 2
        || components[0] != Component::RootDir
        || components[1..]
            .iter()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(denied());
    }
    let mut fd = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    for component in &components[1..components.len() - 1] {
        let s = fstat(&fd)?;
        if (s.st_uid != 0 && s.st_uid != getuid().as_raw()) || s.st_mode & 0o022 != 0 {
            return Err(denied());
        }
        fd = openat(
            &fd,
            component.as_os_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
    }
    let s = fstat(&fd)?;
    if s.st_uid != getuid().as_raw() || s.st_mode & 0o7777 != 0o700 {
        return Err(denied());
    }
    Ok((
        fd,
        components.last().ok_or_else(denied)?.as_os_str().to_owned(),
    ))
}
impl SelectedFile {
    pub(super) fn open(path: PathBuf) -> io::Result<Self> {
        let (parent, name) = parent(&path)?;
        let result = Self {
            path,
            parent,
            name,
            operations: Mutex::new(()),
        };
        result.checked_file()?;
        Ok(result)
    }
    fn check_parent(&self) -> io::Result<()> {
        let (current, _) = parent(&self.path)?;
        if !same(&fstat(&current)?, &fstat(&self.parent)?) {
            return Err(denied());
        }
        Ok(())
    }
    fn checked_file(&self) -> io::Result<Option<(File, Stat)>> {
        self.check_parent()?;
        let fd = match openat(
            &self.parent,
            &self.name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let stat = fstat(&fd)?;
        private_file(&stat)?;
        self.check_named(Some(&stat))?;
        Ok(Some((File::from(fd), stat)))
    }
    fn check_named(&self, expected: Option<&Stat>) -> io::Result<()> {
        self.check_parent()?;
        let current = match statat(&self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(s) => Some(s),
            Err(rustix::io::Errno::NOENT) => None,
            Err(e) => return Err(e.into()),
        };
        match (expected, current) {
            (None, None) => Ok(()),
            (Some(a), Some(b)) if same(a, &b) => private_file(&b),
            _ => Err(denied()),
        }
    }
}
impl AuthStorageBackend for SelectedFile {
    fn load(&self) -> io::Result<Option<AuthDotJson>> {
        let _guard = self
            .operations
            .lock()
            .map_err(|_| io::Error::other("auth-file lock poisoned"))?;
        let Some((mut file, stat)) = self.checked_file()? else {
            return Ok(None);
        };
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        self.check_named(Some(&stat))?;
        Ok(Some(serde_json::from_str(&content)?))
    }
    fn save(&self, auth: &AuthDotJson) -> io::Result<()> {
        let _guard = self
            .operations
            .lock()
            .map_err(|_| io::Error::other("auth-file lock poisoned"))?;
        let previous = self.checked_file()?.map(|(_, s)| s);
        let name = format!(".codex-auth-{:032x}.tmp", rand::random::<u128>());
        let fd = openat(
            &self.parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?;
        let mut file = File::from(fd);
        let result = (|| {
            private_file(&fstat(&file)?)?;
            file.write_all(serde_json::to_string_pretty(auth)?.as_bytes())?;
            file.flush()?;
            self.check_named(previous.as_ref())?;
            renameat(&self.parent, &name, &self.parent, &self.name)?;
            self.check_named(Some(&fstat(&file)?))
        })();
        if result.is_err() {
            let _ = unlinkat(&self.parent, &name, AtFlags::empty());
        }
        result
    }
    fn delete(&self) -> io::Result<bool> {
        let _guard = self
            .operations
            .lock()
            .map_err(|_| io::Error::other("auth-file lock poisoned"))?;
        let Some((_, stat)) = self.checked_file()? else {
            return Ok(false);
        };
        self.check_named(Some(&stat))?;
        unlinkat(&self.parent, &self.name, AtFlags::empty())?;
        self.check_parent()?;
        Ok(true)
    }
}
