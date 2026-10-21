use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::Path;

const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerTemplateValidation {
    pub gui_subsystem: bool,
    pub native_template_marker: bool,
    pub embedded_trusted_public_key: bool,
    pub imported_dlls: Vec<String>,
    pub webview2_dependency: bool,
    pub dynamic_vc_runtime: bool,
    pub powershell_disk_detection: bool,
    pub development_url: bool,
}

pub fn validate_installer_template(
    path: &Path,
    expected_public_key: &[u8; 32],
) -> Result<InstallerTemplateValidation> {
    let bytes =
        fs::read(path).with_context(|| format!("无法读取用户安装器模板: {}", path.display()))?;
    let pe = PeImage::parse(&bytes)?;
    let imported_dlls = pe.imported_dlls()?;
    let lower = ascii_lowercase(&bytes);
    let key_hex = hex::encode(expected_public_key).into_bytes();
    let validation = InstallerTemplateValidation {
        gui_subsystem: pe.subsystem == IMAGE_SUBSYSTEM_WINDOWS_GUI,
        native_template_marker: contains(&bytes, b"YUNSHU_NATIVE_INSTALLER_V1"),
        embedded_trusted_public_key: contains(&lower, &key_hex),
        webview2_dependency: imported_dlls
            .iter()
            .any(|value| value.eq_ignore_ascii_case("webview2loader.dll"))
            || contains(&lower, b"webview2loader.dll")
            || contains(&lower, b"microsoft.web.webview2"),
        dynamic_vc_runtime: imported_dlls.iter().any(|value| {
            let value = value.to_ascii_lowercase();
            value.starts_with("vcruntime140") || value.starts_with("msvcp140")
        }),
        powershell_disk_detection: contains(&lower, b"powershell.exe")
            || contains(&lower, b"get-volume")
            || contains(&lower, b"get-partition")
            || contains(&lower, b"get-disk"),
        development_url: contains(&lower, b"http://localhost:")
            || contains(&lower, b"https://localhost:")
            || contains(&lower, b"http://127.0.0.1:"),
        imported_dlls,
    };

    if !validation.gui_subsystem {
        bail!("用户安装器模板不是Windows GUI子系统");
    }
    if !validation.native_template_marker {
        bail!("用户安装器模板缺少原生模板标识");
    }
    if !validation.embedded_trusted_public_key {
        bail!("用户安装器模板未嵌入当前生产签名公钥");
    }
    if validation.webview2_dependency {
        bail!("用户安装器模板仍依赖WebView2");
    }
    if validation.dynamic_vc_runtime {
        bail!("用户安装器模板仍导入动态VC++ Runtime");
    }
    if validation.powershell_disk_detection {
        bail!("用户安装器模板仍包含PowerShell磁盘检测代码");
    }
    if validation.development_url {
        bail!("用户安装器模板仍包含开发服务器URL");
    }
    Ok(validation)
}

struct PeImage<'a> {
    bytes: &'a [u8],
    subsystem: u16,
    import_rva: u32,
    import_size: u32,
    sections: Vec<Section>,
}

#[derive(Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

impl<'a> PeImage<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.get(..2) != Some(b"MZ") {
            bail!("用户安装器模板不是有效的PE文件");
        }
        let pe_offset = read_u32(bytes, 0x3c)? as usize;
        if bytes.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
            bail!("用户安装器模板缺少PE签名");
        }
        let file_header = pe_offset + 4;
        let section_count = read_u16(bytes, file_header + 2)? as usize;
        let optional_size = read_u16(bytes, file_header + 16)? as usize;
        let optional = file_header + 20;
        let magic = read_u16(bytes, optional)?;
        let data_directory = match magic {
            0x10b => optional + 96,
            0x20b => optional + 112,
            _ => bail!("用户安装器模板的PE可选头格式不受支持"),
        };
        let subsystem = read_u16(bytes, optional + 68)?;
        let import_rva = read_u32(bytes, data_directory + 8)?;
        let import_size = read_u32(bytes, data_directory + 12)?;
        let section_table = optional + optional_size;
        let mut sections = Vec::with_capacity(section_count);
        for index in 0..section_count {
            let offset = section_table
                .checked_add(index.checked_mul(40).context("PE节表范围溢出")?)
                .context("PE节表范围溢出")?;
            sections.push(Section {
                virtual_size: read_u32(bytes, offset + 8)?,
                virtual_address: read_u32(bytes, offset + 12)?,
                raw_size: read_u32(bytes, offset + 16)?,
                raw_offset: read_u32(bytes, offset + 20)?,
            });
        }
        Ok(Self {
            bytes,
            subsystem,
            import_rva,
            import_size,
            sections,
        })
    }

    fn imported_dlls(&self) -> Result<Vec<String>> {
        if self.import_rva == 0 || self.import_size == 0 {
            return Ok(Vec::new());
        }
        let mut offset = self.rva_to_offset(self.import_rva)?;
        let import_end = offset
            .checked_add(self.import_size as usize)
            .unwrap_or(self.bytes.len())
            .min(self.bytes.len());
        let mut imports = Vec::new();
        while offset + 20 <= import_end {
            let descriptor = self
                .bytes
                .get(offset..offset + 20)
                .context("PE导入表被截断")?;
            if descriptor.iter().all(|value| *value == 0) {
                break;
            }
            let name_rva = read_u32(self.bytes, offset + 12)?;
            if name_rva == 0 {
                bail!("PE导入表包含无效DLL名称");
            }
            imports.push(read_ascii_z(
                self.bytes,
                self.rva_to_offset(name_rva)?,
                260,
            )?);
            offset += 20;
        }
        imports.sort_by_key(|value| value.to_ascii_lowercase());
        imports.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        Ok(imports)
    }

    fn rva_to_offset(&self, rva: u32) -> Result<usize> {
        for section in &self.sections {
            let size = section.virtual_size.max(section.raw_size);
            let end = section
                .virtual_address
                .checked_add(size)
                .context("PE节范围溢出")?;
            if rva >= section.virtual_address && rva < end {
                let delta = rva - section.virtual_address;
                let offset = section
                    .raw_offset
                    .checked_add(delta)
                    .context("PE文件偏移溢出")? as usize;
                if offset >= self.bytes.len() {
                    bail!("PE RVA超出文件范围");
                }
                return Ok(offset);
            }
        }
        bail!("PE RVA未映射到任何节: 0x{rva:08x}")
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value: [u8; 2] = bytes
        .get(offset..offset + 2)
        .context("PE文件被截断")?
        .try_into()
        .unwrap();
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .context("PE文件被截断")?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(value))
}

fn read_ascii_z(bytes: &[u8], offset: usize, max: usize) -> Result<String> {
    let bytes = bytes.get(offset..).context("PE字符串偏移超出范围")?;
    let length = bytes
        .iter()
        .take(max)
        .position(|value| *value == 0)
        .context("PE字符串缺少终止符")?;
    let value = std::str::from_utf8(&bytes[..length]).context("PE DLL名称不是ASCII")?;
    Ok(value.to_owned())
}

fn ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|value| value == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pe_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("template.exe");
        fs::write(&path, b"not-a-pe").unwrap();
        assert!(validate_installer_template(&path, &[1; 32]).is_err());
    }

    #[test]
    fn byte_search_is_ascii_case_insensitive_after_normalization() {
        let value = ascii_lowercase(b"VCRUNTIME140_1.DLL");
        assert!(contains(&value, b"vcruntime140"));
    }
}
