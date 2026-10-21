use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const INSTALLER_MAGIC: [u8; 16] = *b"YUNSHU_USB_V1\0\0\0";
pub const FOOTER_SIZE: usize = 164;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerFooterV1 {
    pub schema_version: u32,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub manifest_offset: u64,
    pub manifest_length: u64,
    pub signature_offset: u64,
    pub signature_length: u32,
    pub payload_sha256: [u8; 32],
    pub manifest_sha256: [u8; 32],
    pub public_key: [u8; 32],
}

impl InstallerFooterV1 {
    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut out = [0_u8; FOOTER_SIZE];
        let mut cursor = 0;
        put(&mut out, &mut cursor, &INSTALLER_MAGIC);
        put(&mut out, &mut cursor, &self.schema_version.to_le_bytes());
        put(&mut out, &mut cursor, &self.payload_offset.to_le_bytes());
        put(&mut out, &mut cursor, &self.payload_length.to_le_bytes());
        put(&mut out, &mut cursor, &self.manifest_offset.to_le_bytes());
        put(&mut out, &mut cursor, &self.manifest_length.to_le_bytes());
        put(&mut out, &mut cursor, &self.signature_offset.to_le_bytes());
        put(&mut out, &mut cursor, &self.signature_length.to_le_bytes());
        put(&mut out, &mut cursor, &(FOOTER_SIZE as u32).to_le_bytes());
        put(&mut out, &mut cursor, &self.payload_sha256);
        put(&mut out, &mut cursor, &self.manifest_sha256);
        put(&mut out, &mut cursor, &self.public_key);
        debug_assert_eq!(cursor, FOOTER_SIZE);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != FOOTER_SIZE {
            bail!("安装器尾部长度不正确");
        }
        let mut cursor = 0;
        let magic = take::<16>(bytes, &mut cursor);
        if magic != INSTALLER_MAGIC {
            bail!("未找到云枢安装器尾部标识");
        }
        let schema_version = u32::from_le_bytes(take(bytes, &mut cursor));
        if schema_version != 1 {
            bail!("不支持的安装器 schemaVersion: {schema_version}");
        }
        let payload_offset = u64::from_le_bytes(take(bytes, &mut cursor));
        let payload_length = u64::from_le_bytes(take(bytes, &mut cursor));
        let manifest_offset = u64::from_le_bytes(take(bytes, &mut cursor));
        let manifest_length = u64::from_le_bytes(take(bytes, &mut cursor));
        let signature_offset = u64::from_le_bytes(take(bytes, &mut cursor));
        let signature_length = u32::from_le_bytes(take(bytes, &mut cursor));
        let footer_length = u32::from_le_bytes(take(bytes, &mut cursor));
        if footer_length as usize != FOOTER_SIZE {
            bail!("安装器尾部版本不匹配");
        }
        let payload_sha256 = take(bytes, &mut cursor);
        let manifest_sha256 = take(bytes, &mut cursor);
        let public_key = take(bytes, &mut cursor);
        Ok(Self {
            schema_version,
            payload_offset,
            payload_length,
            manifest_offset,
            manifest_length,
            signature_offset,
            signature_length,
            payload_sha256,
            manifest_sha256,
            public_key,
        })
    }

    pub fn validate_bounds(&self, file_length: u64) -> Result<()> {
        let footer_start = file_length
            .checked_sub(FOOTER_SIZE as u64)
            .ok_or_else(|| anyhow::anyhow!("安装器文件过短"))?;
        let payload_end = self
            .payload_offset
            .checked_add(self.payload_length)
            .ok_or_else(|| anyhow::anyhow!("载荷范围溢出"))?;
        let manifest_end = self
            .manifest_offset
            .checked_add(self.manifest_length)
            .ok_or_else(|| anyhow::anyhow!("清单范围溢出"))?;
        let signature_end = self
            .signature_offset
            .checked_add(self.signature_length as u64)
            .ok_or_else(|| anyhow::anyhow!("签名范围溢出"))?;
        if self.payload_offset == 0
            || payload_end != self.manifest_offset
            || manifest_end != self.signature_offset
            || signature_end != footer_start
            || self.signature_length != 64
        {
            bail!("安装器内部区段不连续或已损坏");
        }
        Ok(())
    }
}

fn put(out: &mut [u8], cursor: &mut usize, bytes: &[u8]) {
    out[*cursor..*cursor + bytes.len()].copy_from_slice(bytes);
    *cursor += bytes.len();
}

fn take<const N: usize>(bytes: &[u8], cursor: &mut usize) -> [u8; N] {
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes[*cursor..*cursor + N]);
    *cursor += N;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_round_trip_and_bounds() {
        let footer = InstallerFooterV1 {
            schema_version: 1,
            payload_offset: 100,
            payload_length: 200,
            manifest_offset: 300,
            manifest_length: 80,
            signature_offset: 380,
            signature_length: 64,
            payload_sha256: [1; 32],
            manifest_sha256: [2; 32],
            public_key: [3; 32],
        };
        let decoded = InstallerFooterV1::decode(&footer.encode()).unwrap();
        assert_eq!(decoded, footer);
        decoded.validate_bounds(444 + FOOTER_SIZE as u64).unwrap();
    }

    #[test]
    fn footer_rejects_discontinuous_sections() {
        let footer = InstallerFooterV1 {
            schema_version: 1,
            payload_offset: 100,
            payload_length: 199,
            manifest_offset: 300,
            manifest_length: 80,
            signature_offset: 380,
            signature_length: 64,
            payload_sha256: [0; 32],
            manifest_sha256: [0; 32],
            public_key: [0; 32],
        };
        assert!(footer.validate_bounds(444 + FOOTER_SIZE as u64).is_err());
    }
}
