use crate::interpreter::value::{HashKey, Value};

/// Serialize a Value to a JSON string using sonic-rs SIMD-accelerated writer.
#[inline]
pub fn stringify_to_string(value: &Value) -> Result<String, String> {
    #[cfg(feature = "sonic-rs")]
    {
        let bytes = sonic_rs::to_vec(value).map_err(|e| e.to_string())?;
        Ok(unsafe { String::from_utf8_unchecked(bytes) })
    }
    #[cfg(not(feature = "sonic-rs"))]
    {
        serde_json::to_string(value).map_err(|e| e.to_string())
    }
}

/// Serialize an array slice to JSON without cloning into a Value.
#[inline]
pub fn stringify_array_to_string(items: &[Value]) -> Result<String, String> {
    #[cfg(feature = "sonic-rs")]
    {
        let bytes = sonic_rs::to_vec(items).map_err(|e| e.to_string())?;
        Ok(unsafe { String::from_utf8_unchecked(bytes) })
    }
    #[cfg(not(feature = "sonic-rs"))]
    {
        serde_json::to_string(items).map_err(|e| e.to_string())
    }
}

pub struct HashEntrySlice<'a>(pub &'a [(HashKey, Value)]);

impl serde::Serialize for HashEntrySlice<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            if let HashKey::String(key) = k {
                map.serialize_entry(key, v)?;
            }
        }
        map.end()
    }
}

/// Serialize hash entries to JSON without cloning into a Value.
#[inline]
pub fn stringify_hash_entries_to_string(entries: &[(HashKey, Value)]) -> Result<String, String> {
    #[cfg(feature = "sonic-rs")]
    {
        let bytes = sonic_rs::to_vec(&HashEntrySlice(entries)).map_err(|e| e.to_string())?;
        Ok(unsafe { String::from_utf8_unchecked(bytes) })
    }
    #[cfg(not(feature = "sonic-rs"))]
    {
        serde_json::to_string(&HashEntrySlice(entries)).map_err(|e| e.to_string())
    }
}
