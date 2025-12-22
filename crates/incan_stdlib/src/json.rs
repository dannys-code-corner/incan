//! JSON serialization support for Incan types.
//!
//! Provides convenient wrappers around serde_json for types that implement
//! `Serialize` and `Deserialize`.

use serde::{Deserialize, Serialize};
use std::error::Error;

/// Trait for types that can be serialized to JSON.
///
/// This is automatically implemented for any type that implements `serde::Serialize`.
pub trait ToJson: Serialize {
    /// Serializes this value to a JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    fn to_json(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(self).map_err(|e| Box::new(e) as Box<dyn Error>)
    }

    /// Serializes this value to a pretty-printed JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    fn to_json_pretty(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string_pretty(self).map_err(|e| Box::new(e) as Box<dyn Error>)
    }
}

/// Trait for types that can be deserialized from JSON.
///
/// This is automatically implemented for any type that implements `serde::Deserialize`.
pub trait FromJson: for<'de> Deserialize<'de> {
    /// Deserializes a value from a JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails.
    fn from_json(json: &str) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized,
    {
        serde_json::from_str(json).map_err(|e| Box::new(e) as Box<dyn Error>)
    }
}

// Blanket implementations for all types that implement the required serde traits
impl<T: Serialize> ToJson for T {}
impl<T: for<'de> Deserialize<'de>> FromJson for T {}
