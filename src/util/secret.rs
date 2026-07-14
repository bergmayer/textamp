use std::fmt;
use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

/// Editable secret text that wipes its allocation on drop and never exposes
/// its contents through `Debug` output.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct SecretString(String);

impl SecretString {
    /// Move the secret out for immediate transfer into another zeroizing
    /// owner, leaving this value empty.
    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl serde::Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Deref for SecretString {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SecretString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Zeroize for SecretString {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_is_redacted() {
        let mut secret = SecretString::default();
        secret.push_str("not-for-logs");

        let output = format!("{secret:?}");
        assert!(!output.contains("not-for-logs"));
        assert!(output.contains("REDACTED"));
    }

    #[test]
    fn take_leaves_the_edit_buffer_empty() {
        let mut secret = SecretString::default();
        secret.push_str("password");

        let moved = secret.take();
        assert_eq!(moved, "password");
        assert!(secret.is_empty());
    }
}
