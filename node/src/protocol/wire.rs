use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::ops::Deref;
use std::string::FromUtf8Error;
use std::{io, mem};

use byteorder::{NetworkEndian, ReadBytesExt, WriteBytesExt};

use crate::crypto::{PublicKey, Signature};
use crate::git;
use crate::git::fmt;
use crate::hash::Digest;
use crate::identity::Id;
use crate::storage::refs::Refs;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("UTF-8 error: {0}")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("invalid size: expected {expected}, got {actual}")]
    InvalidSize { expected: usize, actual: usize },
    #[error(transparent)]
    InvalidRefName(#[from] fmt::Error),
    #[error("invalid git url `{url}`: {error}")]
    InvalidGitUrl {
        url: String,
        error: git::url::parse::Error,
    },
    #[error("unknown address type `{0}`")]
    UnknownAddressType(u8),
    #[error("unknown message type `{0}`")]
    UnknownMessageType(u16),
}

impl Error {
    /// Whether we've reached the end of file. This will be true when we fail to decode
    /// a message because there's not enough data in the stream.
    pub fn is_eof(&self) -> bool {
        matches!(self, Self::Io(err) if err.kind() == io::ErrorKind::UnexpectedEof)
    }
}

/// Things that can be encoded as binary.
pub trait Encode {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error>;
}

/// Things that can be decoded from binary.
pub trait Decode: Sized {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error>;
}

/// Encode an object into a vector.
pub fn serialize<T: Encode + ?Sized>(data: &T) -> Vec<u8> {
    let mut buffer = Vec::new();
    let len = data
        .encode(&mut buffer)
        .expect("in-memory writes don't error");

    debug_assert_eq!(len, buffer.len());

    buffer
}

/// Decode an object from a vector.
pub fn deserialize<T: Decode>(data: &[u8]) -> Result<T, Error> {
    let mut cursor = io::Cursor::new(data);

    T::decode(&mut cursor)
}

impl Encode for u8 {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_u8(*self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl Encode for u16 {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_u16::<NetworkEndian>(*self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl Encode for u32 {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_u32::<NetworkEndian>(*self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl Encode for u64 {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_u64::<NetworkEndian>(*self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl Encode for usize {
    /// We encode this type to a [`u32`], since there's no need to send larger messages
    /// over the network.
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        assert!(
            *self <= u32::MAX as usize,
            "Cannot encode sizes larger than {}",
            u32::MAX
        );
        writer.write_u32::<NetworkEndian>(*self as u32)?;

        Ok(mem::size_of::<u32>())
    }
}

impl Encode for PublicKey {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.as_bytes().encode(writer)
    }
}

impl<const T: usize> Encode for &[u8; T] {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_all(*self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl<const T: usize> Encode for [u8; T] {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        writer.write_all(self)?;

        Ok(mem::size_of::<Self>())
    }
}

impl<T> Encode for &[T]
where
    T: Encode,
{
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        let mut n = self.len().encode(writer)?;

        for item in self.iter() {
            n += item.encode(writer)?;
        }
        Ok(n)
    }
}

impl Encode for &str {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        assert!(self.len() <= u8::MAX as usize);

        let n = (self.len() as u8).encode(writer)?;
        let bytes = self.as_bytes();

        // Nb. Don't use the [`Encode`] instance here for &[u8], because we are prefixing the
        // length ourselves.
        writer.write_all(bytes)?;

        Ok(n + bytes.len())
    }
}

impl Encode for String {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.as_str().encode(writer)
    }
}

impl Encode for git::Url {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.to_string().encode(writer)
    }
}

impl Encode for Digest {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.as_ref().encode(writer)
    }
}

impl Encode for Id {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.deref().encode(writer)
    }
}

impl Encode for Refs {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        let mut n = self.len().encode(writer)?;

        for (name, oid) in self.iter() {
            n += name.as_str().encode(writer)?;
            n += oid.encode(writer)?;
        }
        Ok(n)
    }
}

impl Encode for Signature {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.to_bytes().encode(writer)
    }
}

impl Encode for git::Oid {
    fn encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        // Nb. We use length-encoding here to support future SHA-2 object ids.
        self.as_bytes().encode(writer)
    }
}

////////////////////////////////////////////////////////////////////////////////

impl Decode for PublicKey {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let buf: [u8; 32] = Decode::decode(reader)?;

        PublicKey::try_from(buf)
            .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::InvalidInput, e.to_string())))
    }
}

impl Decode for Refs {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let len = usize::decode(reader)?;
        let mut refs = BTreeMap::new();

        for _ in 0..len {
            let name = String::decode(reader)?;
            let name = git::RefString::try_from(name).map_err(Error::from)?;
            let oid = git::Oid::decode(reader)?;

            refs.insert(name, oid);
        }
        Ok(refs.into())
    }
}

impl Decode for git::Oid {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let len = usize::decode(reader)?;
        #[allow(non_upper_case_globals)]
        const expected: usize = mem::size_of::<git2::Oid>();

        if len != expected {
            return Err(Error::InvalidSize {
                expected,
                actual: len,
            });
        }

        let buf: [u8; expected] = Decode::decode(reader)?;
        let oid = git2::Oid::from_bytes(&buf).expect("the buffer is exactly the right size");
        let oid = git::Oid::from(oid);

        Ok(oid)
    }
}

impl Decode for Signature {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let bytes: [u8; 64] = Decode::decode(reader)?;

        Ok(Signature::from(bytes))
    }
}

impl Decode for u8 {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        reader.read_u8().map_err(Error::from)
    }
}

impl Decode for u16 {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        reader.read_u16::<NetworkEndian>().map_err(Error::from)
    }
}

impl Decode for u32 {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        reader.read_u32::<NetworkEndian>().map_err(Error::from)
    }
}

impl Decode for u64 {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        reader.read_u64::<NetworkEndian>().map_err(Error::from)
    }
}

impl Decode for usize {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let size: usize = u32::decode(reader)?
            .try_into()
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;

        Ok(size)
    }
}

impl<const N: usize> Decode for [u8; N] {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let mut ary = [0; N];
        reader.read_exact(&mut ary)?;

        Ok(ary)
    }
}

impl<T> Decode for Vec<T>
where
    T: Decode,
{
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let len: usize = usize::decode(reader)?;
        let mut vec = Vec::with_capacity(len);

        for _ in 0..len {
            let item = T::decode(reader)?;
            vec.push(item);
        }
        Ok(vec)
    }
}

impl Decode for String {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let len = u8::decode(reader)?;
        let mut bytes = vec![0; len as usize];

        reader.read_exact(&mut bytes)?;

        let string = String::from_utf8(bytes)?;

        Ok(string)
    }
}

impl Decode for git::Url {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let url = String::decode(reader)?;
        let url = Self::from_bytes(url.as_bytes())
            .map_err(|error| Error::InvalidGitUrl { url, error })?;

        Ok(url)
    }
}

impl Decode for Id {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let digest: Digest = Decode::decode(reader)?;

        Ok(Self::from(digest))
    }
}

impl Decode for Digest {
    fn decode<R: io::Read + ?Sized>(reader: &mut R) -> Result<Self, Error> {
        let bytes: [u8; 32] = Decode::decode(reader)?;

        Ok(Self::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck_macros::quickcheck;

    use crate::crypto::Unverified;
    use crate::storage::refs::SignedRefs;
    use crate::test::arbitrary;

    #[quickcheck]
    fn prop_u8(input: u8) {
        assert_eq!(deserialize::<u8>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_u16(input: u16) {
        assert_eq!(deserialize::<u16>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_u32(input: u32) {
        assert_eq!(deserialize::<u32>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_u64(input: u64) {
        assert_eq!(deserialize::<u64>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_usize(input: usize) -> quickcheck::TestResult {
        if input > u32::MAX as usize {
            return quickcheck::TestResult::discard();
        }
        assert_eq!(deserialize::<usize>(&serialize(&input)).unwrap(), input);

        quickcheck::TestResult::passed()
    }

    #[quickcheck]
    fn prop_string(input: String) -> quickcheck::TestResult {
        if input.len() > u8::MAX as usize {
            return quickcheck::TestResult::discard();
        }
        assert_eq!(deserialize::<String>(&serialize(&input)).unwrap(), input);

        quickcheck::TestResult::passed()
    }

    #[quickcheck]
    fn prop_pubkey(input: PublicKey) {
        assert_eq!(deserialize::<PublicKey>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_id(input: Id) {
        assert_eq!(deserialize::<Id>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_digest(input: Digest) {
        assert_eq!(deserialize::<Digest>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_refs(input: Refs) {
        assert_eq!(deserialize::<Refs>(&serialize(&input)).unwrap(), input);
    }

    #[quickcheck]
    fn prop_signature(input: arbitrary::ByteArray<64>) {
        let signature = Signature::from(input.into_inner());

        assert_eq!(
            deserialize::<Signature>(&serialize(&signature)).unwrap(),
            signature
        );
    }

    #[quickcheck]
    fn prop_oid(input: arbitrary::ByteArray<20>) {
        let oid = git::Oid::try_from(input.into_inner().as_slice()).unwrap();

        assert_eq!(deserialize::<git::Oid>(&serialize(&oid)).unwrap(), oid);
    }

    #[quickcheck]
    fn prop_signed_refs(input: SignedRefs<Unverified>) {
        assert_eq!(
            deserialize::<SignedRefs<Unverified>>(&serialize(&input)).unwrap(),
            input
        );
    }

    #[test]
    fn test_string() {
        assert_eq!(
            serialize(&String::from("hello")),
            vec![5, b'h', b'e', b'l', b'l', b'o']
        );
    }

    #[test]
    fn test_git_url() {
        let url = git::Url {
            scheme: git::url::Scheme::Https,
            path: "/git".to_owned().into(),
            host: Some("seed.radicle.xyz".to_owned()),
            port: Some(8888),
            ..git::Url::default()
        };
        assert_eq!(deserialize::<git::Url>(&serialize(&url)).unwrap(), url);
    }
}
