//! Serde helper functions for flexible deserialization.
//!
//! The Plex API sometimes returns numbers as strings, so we need
//! flexible deserializers that can handle both.

use serde::{de, Deserialize, Deserializer};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// Deserialize an optional number that may come as either a string or number.
pub fn from_str_or_num_opt<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: FromStr + Deserialize<'de>,
    T::Err: fmt::Display,
    D: Deserializer<'de>,
{
    struct StringOrNumOpt<T>(PhantomData<T>);

    impl<'de, T> de::Visitor<'de> for StringOrNumOpt<T>
    where
        T: FromStr + Deserialize<'de>,
        T::Err: fmt::Display,
    {
        type Value = Option<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("null, a string, or a number")
        }

        fn visit_none<E>(self) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_str<E>(self, value: &str) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                Ok(None)
            } else {
                T::from_str(value).map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_u64<E>(self, value: u64) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            T::from_str(&value.to_string())
                .map(Some)
                .map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            T::from_str(&value.to_string())
                .map(Some)
                .map_err(de::Error::custom)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Option<T>, E>
        where
            E: de::Error,
        {
            T::from_str(&value.to_string())
                .map(Some)
                .map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(StringOrNumOpt(PhantomData))
}
