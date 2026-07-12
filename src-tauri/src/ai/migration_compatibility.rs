// pattern: Functional Core

use sha2::{Digest, Sha384};

const DREAM_MEMORY_V2_SQL: &[u8] = include_bytes!("../../migrations/0008_dream_memory_v2.sql");

#[derive(Debug, PartialEq, Eq)]
pub enum ChecksumKind {
    Current,
    CompatibleLineEndingVariant,
    Unknown,
}

pub struct DreamMemoryV2Checksums {
    pub current: [u8; 48],
    pub crlf: [u8; 48],
}

pub fn dream_memory_v2_checksums() -> DreamMemoryV2Checksums {
    let current = Sha384::digest(DREAM_MEMORY_V2_SQL).into();
    let mut crlf_sql = Vec::with_capacity(DREAM_MEMORY_V2_SQL.len());

    for (index, byte) in DREAM_MEMORY_V2_SQL.iter().copied().enumerate() {
        if byte == b'\n' && (index == 0 || DREAM_MEMORY_V2_SQL[index - 1] != b'\r') {
            crlf_sql.push(b'\r');
        }
        crlf_sql.push(byte);
    }

    DreamMemoryV2Checksums {
        current,
        crlf: Sha384::digest(&crlf_sql).into(),
    }
}

pub fn classify_dream_memory_v2_checksum(
    checksum: &[u8],
    checksums: &DreamMemoryV2Checksums,
) -> ChecksumKind {
    if checksum == checksums.current {
        ChecksumKind::Current
    } else if checksum == checksums.crlf {
        ChecksumKind::CompatibleLineEndingVariant
    } else {
        ChecksumKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_dream_memory_v2_checksum, dream_memory_v2_checksums, ChecksumKind};

    #[test]
    fn accepts_lf_and_crlf_variants_of_the_published_migration() {
        let checksums = dream_memory_v2_checksums();

        assert_eq!(
            classify_dream_memory_v2_checksum(&checksums.current, &checksums),
            ChecksumKind::Current
        );
        assert_eq!(
            classify_dream_memory_v2_checksum(&checksums.crlf, &checksums),
            ChecksumKind::CompatibleLineEndingVariant
        );
        assert_eq!(
            classify_dream_memory_v2_checksum(&[0; 48], &checksums),
            ChecksumKind::Unknown
        );
    }
}
