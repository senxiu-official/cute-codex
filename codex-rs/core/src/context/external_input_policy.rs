//! Receiver-owned policy, loaded once by the trusted launch path. Not a token count.
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error;
use serde::de::Visitor;
use std::fmt;
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalBytePolicy {
    Limit(NonZeroU32),
    Off,
}

impl Default for CanonicalBytePolicy {
    #[expect(
        clippy::expect_used,
        reason = "the fixed default is a positive constant"
    )]
    fn default() -> Self {
        Self::Limit(NonZeroU32::new(10_000).expect("positive default byte limit"))
    }
}

impl CanonicalBytePolicy {
    pub(crate) fn permits(self, bytes: usize) -> bool {
        match self {
            Self::Limit(limit) => bytes as u64 <= u64::from(limit.get()),
            Self::Off => true,
        }
    }
}

impl<'de> Deserialize<'de> for CanonicalBytePolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PolicyVisitor;
        impl Visitor<'_> for PolicyVisitor {
            type Value = CanonicalBytePolicy;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an integer byte limit in 1..=4294967295 or the string off")
            }
            fn visit_u64<E: Error>(self, value: u64) -> Result<Self::Value, E> {
                u32::try_from(value)
                    .ok()
                    .and_then(NonZeroU32::new)
                    .map(CanonicalBytePolicy::Limit)
                    .ok_or_else(|| E::custom("canonicalByteLimit is out of range"))
            }
            fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
                if value == "off" {
                    Ok(CanonicalBytePolicy::Off)
                } else {
                    Err(E::custom(
                        "canonicalByteLimit expects a positive integer or off",
                    ))
                }
            }
        }
        deserializer.deserialize_any(PolicyVisitor)
    }
}
