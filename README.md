<!-- cute-codex: this section is added by the fork; the upstream Codex README follows below. -->

# cute-codex

cute-codex is a fork of [OpenAI Codex](https://github.com/openai/codex) used as the agent runtime
of [cutex](https://github.com/senxiu-official/cutex) and
[Cutex desktop](https://github.com/senxiu-official/cutex-desktop). It adds external input into
running threads, presentations, selected auth files and a few fixes. Every modified or added file
is listed in [CUTE-CODEX-CHANGES.md](CUTE-CODEX-CHANGES.md). It is not an official OpenAI release.

- **Releases:** the [`release` branch](https://github.com/senxiu-official/cute-codex/tree/release)
  has one commit per release on top of the upstream tag it is based on, tagged
  `cute-codex-v<version>`. See [Releases](https://github.com/senxiu-official/cute-codex/releases).
  The `main` branch is an unmodified mirror of upstream.
- **Install:** you normally do not install this on its own. The cutex install guide
  ([AGENT-INSTALL.md](https://github.com/senxiu-official/cutex-desktop/blob/release/docs/install/AGENT-INSTALL.md))
  either unpacks the release binaries (section B) or builds it from the `release` branch (section 3).
- **Release binaries:** each release carries `native-<version>-linux-x64.tar.gz` and
  `-windows-x64.tar.gz` (`codex-app-server`, `codex`, `codex-code-mode-host` and the app-server
  protocol schema), built by GitHub Actions (`.github/workflows/cute-codex-build-binaries.yml`) and
  signed with minisign (`<file>.minisig`; public key
  `RWRH9INhAb2gACJAg/h5EBjbudos2LXj6K+5hRBEGeVLDKaKzE+FICmS`). No macOS build: cutex agents do not
  run on macOS.
- **License:** Apache-2.0, like upstream ([LICENSE](LICENSE), [NOTICE](NOTICE)).

Built with Codex, Claude, DeepSeek, GLM and Kimi. Product names are trademarks of their owners;
naming them implies no affiliation or endorsement.

---

<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>
</br>
If you want Codex in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>codex app</code> or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## Quickstart

### Installing and running Codex CLI

Run the following on Mac or Linux to install Codex CLI:

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

Run the following on Windows to install Codex CLI:

```shell
powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"
```

The standalone installers download from `https://releases.openai.com/codex` by default and fall back to GitHub Releases if a metadata or asset download is unavailable. To force GitHub Releases, set `CODEX_INSTALLER_USE_RELEASES_OPENAI_COM` to `false` (`0` and `no` are also accepted):

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_INSTALLER_USE_RELEASES_OPENAI_COM=false sh
```

```powershell
$env:CODEX_INSTALLER_USE_RELEASES_OPENAI_COM='false'; irm https://chatgpt.com/codex/install.ps1 | iex
```

Codex CLI can also be installed via the following package managers:

```shell
# Install using npm
npm install -g @openai/codex
```

```shell
# Install using Homebrew
brew install --cask codex
```

Then simply run `codex` to get started.

<details>
<summary>You can also go to the <a href="https://github.com/openai/codex/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

Each archive contains a single entry with the platform baked into the name (e.g., `codex-x86_64-unknown-linux-musl`), so you likely want to rename it to `codex` after extracting it.

</details>

### Using Codex with your ChatGPT plan

Run `codex` and select **Sign in with ChatGPT**. We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Codex with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Codex Documentation**](https://developers.openai.com/codex)
- [**Contributing**](./docs/contributing.md)
- [**Installing & building**](./docs/install.md)
- [**Open source fund**](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
