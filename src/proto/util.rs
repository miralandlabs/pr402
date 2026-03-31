//! Protocol utility types.

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct U16String(u16);

impl U16String {
    pub fn inner(&self) -> u16 {
        self.0
    }
}

impl From<u16> for U16String {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<U16String> for u16 {
    fn from(value: U16String) -> Self {
        value.0
    }
}

impl Serialize for U16String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U16String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.trim().replace(',', "");
        if s.contains('.') {
            if let Ok(f) = s.parse::<f64>() {
                if f.fract() == 0.0 && f >= 0.0 && f <= (u16::MAX as f64) {
                    return Ok(Self(f as u16));
                }
            }
        }
        s.parse::<u16>().map(Self).map_err(Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct U64String(u64);

impl U64String {
    pub fn inner(&self) -> u64 {
        self.0
    }
}

impl From<u64> for U64String {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<U64String> for u64 {
    fn from(value: U64String) -> Self {
        value.0
    }
}

impl Serialize for U64String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U64String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.trim().replace(',', "");
        // If it looks like a float (e.g., "1.0"), try parsing as f64 first to handle trailing zeros
        if s.contains('.') {
            if let Ok(f) = s.parse::<f64>() {
                if f.fract() == 0.0 && f >= 0.0 && f <= (u64::MAX as f64) {
                    return Ok(Self(f as u64));
                }
            }
        }
        // Fallback or standard integer path
        s.parse::<u64>().map(Self).map_err(Error::custom)
    }
}
