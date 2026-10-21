mod footer;
mod manifest;
mod package;
mod targets;
mod template;
mod validation;
mod writer;

pub use footer::{FOOTER_SIZE, INSTALLER_MAGIC, InstallerFooterV1};
pub use manifest::{
    FileScope, HostCompatibility, PayloadArchitecture, ReleaseDirectory, ReleaseFile,
    ReleaseManifest, RootFileMapping, build_release_manifest,
};
pub use package::{
    VerifiedInstaller, build_single_exe_installer, load_or_create_signing_key, load_signing_key,
    public_key_from_hex, verify_single_exe_installer, verify_single_exe_installer_deep,
    verify_single_exe_installer_deep_with_public_key, verify_single_exe_installer_with_public_key,
};
pub use targets::{TargetDiskInfo, list_usb_targets};
pub use template::{InstallerTemplateValidation, validate_installer_template};
pub use validation::{FactorySummary, MasterCheck, MasterValidationResult, validate_master};
pub use writer::{
    InstallRequest, WriteJobStatus, WriteOperation, WritePhase, cleanup_owned_staging,
    install_from_master, install_from_verified_installer, install_from_verified_installer_checked,
};

pub const DEFAULT_PRODUCT_DIRECTORY: &str = "云枢AI便携工作站";
pub const DELIVERY_SOURCE_DIRECTORY: &str = "交付资料/写入U盘根目录";
