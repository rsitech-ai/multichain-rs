use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};

/// Exact Bitcoin amount in satoshis.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sats(u64);

impl Sats {
    /// Constructs an exact satoshi amount.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact integer amount.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Checked addition that never silently wraps.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for Sats {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for Sats {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Sats {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalStringVisitor;

        impl Visitor<'_> for DecimalStringVisitor {
            type Value = Sats;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an unsigned u64 decimal string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value
                    .parse::<u64>()
                    .map(Sats)
                    .map_err(|_| E::custom("amount is outside u64"))
            }
        }

        deserializer.deserialize_str(DecimalStringVisitor)
    }
}
