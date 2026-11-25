use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Hash BLAKE3 de 32 bytes, mostrado em hexadecimal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash([u8; 32]);

impl Hash {
    pub const fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    /// BLAKE3 de `data`.
    pub fn of(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        const D: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(D[(b >> 4) as usize] as char);
            s.push(D[(b & 15) as usize] as char);
        }
        s
    }

    /// Os primeiros 10 caracteres, para mostrar ao usuário.
    pub fn short(&self) -> String {
        self.to_hex()[..10].to_string()
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Texto que não é um hash hexadecimal de 64 caracteres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashParseError;

impl fmt::Display for HashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("hash inválido: esperado hexadecimal de 64 caracteres")
    }
}

impl std::error::Error for HashParseError {}

impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.as_bytes();
        if s.len() != 64 {
            return Err(HashParseError);
        }
        let mut out = [0u8; 32];
        for (i, pair) in s.chunks(2).enumerate() {
            let hi = (pair[0] as char).to_digit(16).ok_or(HashParseError)?;
            let lo = (pair[1] as char).to_digit(16).ok_or(HashParseError)?;
            out[i] = (hi * 16 + lo) as u8;
        }
        Ok(Self(out))
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_ida_e_volta() {
        let h = Hash::of(b"relicario");
        assert_eq!(h.to_hex().parse::<Hash>().unwrap(), h);
        assert!("xyz".parse::<Hash>().is_err());
    }
}
