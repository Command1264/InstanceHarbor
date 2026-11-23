use std::{
    ffi::{c_void, OsStr},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut, NonNull},
};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const FILE_OPEN_DIALOG: Guid = Guid(
    0xdc1c5a9c, 0xe88a, 0x4dde, [0xa5, 0xa1, 0x60, 0xf8, 0x2a, 0x20, 0xae, 0xf7],
);
const FILE_SAVE_DIALOG: Guid = Guid(
    0xc0b4e2f3, 0xba21, 0x4773, [0x8d, 0xba, 0x33, 0x5e, 0xc9, 0x46, 0xeb, 0x8b],
);
const FILE_DIALOG: Guid = Guid(
    0x42f85136, 0xdb7e, 0x439c, [0x85, 0xf1, 0xe4, 0x07, 0x5d, 0x13, 0x5f, 0xc8],
);
const SHELL_ITEM: Guid = Guid(
    0x43826d1e, 0xe718, 0x42ee, [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe],
);
const PICK_FOLDERS: u32 = 0x20;
const FORCE_FILE_SYSTEM: u32 = 0x40;
const PATH_MUST_EXIST: u32 = 0x800;
const FILE_MUST_EXIST: u32 = 0x1000;
const NO_RECENT: u32 = 0x0200_0000;
const FILE_SYSTEM_PATH: u32 = 0x8005_8000;
const CANCELLED: i32 = 0x8007_04c7_u32 as i32;

#[repr(C)]
struct FileType {
    name: *const u16,
    pattern: *const u16,
}

#[derive(Clone, Copy)]
pub enum Pick {
    Folder,
    Jar,
    Java,
    Json,
    Recovery,
    SaveRecovery,
    File,
}

impl Pick {
    pub fn file(kind: &str) -> Result<Self, String> {
        match kind {
            "jar" => Ok(Self::Jar),
            "java" => Ok(Self::Java),
            "json" => Ok(Self::Json),
            "recovery" => Ok(Self::Recovery),
            "save-recovery" => Ok(Self::SaveRecovery),
            "any" => Ok(Self::File),
            _ => Err("InvalidPickerKind".into()),
        }
    }

    fn options(self) -> u32 {
        FORCE_FILE_SYSTEM | PATH_MUST_EXIST | NO_RECENT | match self {
            Self::Folder => PICK_FOLDERS,
            Self::SaveRecovery => 0,
            _ => FILE_MUST_EXIST,
        }
    }

    fn filter(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Folder => None,
            Self::Jar => Some(("JAR", "*.jar")),
            Self::Java => Some(("java.exe", "java.exe")),
            Self::Json => Some(("JSON", "*.json")),
            Self::Recovery | Self::SaveRecovery => Some(("InstanceHarbor", "*.ihrecovery")),
            Self::File => None,
        }
    }
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, mode: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        class: *const Guid,
        outer: *mut c_void,
        context: u32,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> i32;
    fn CoTaskMemFree(value: *mut c_void);
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHCreateItemFromParsingName(
        path: *const u16,
        context: *const c_void,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> i32;
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

struct Interface(NonNull<c_void>);
impl Interface {
    fn from_call(result: i32, object: *mut c_void) -> Result<Self, String> {
        check(result)?;
        NonNull::new(object)
            .map(Self)
            .ok_or_else(|| "FolderPickerUnavailable".into())
    }

    // All slot numbers are from the Windows SDK IUnknown, IFileDialog
    // and IShellItem declarations, including inherited entries.
    unsafe fn slot(&self, index: usize) -> *const c_void {
        unsafe { *(*self.0.as_ptr().cast::<*const *const c_void>()).add(index) }
    }
}
impl Drop for Interface {
    fn drop(&mut self) {
        unsafe {
            let release: extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(self.slot(2));
            release(self.0.as_ptr());
        }
    }
}

fn check(result: i32) -> Result<(), String> {
    if result < 0 {
        return Err(format!("FolderPickerUnavailable:{:08x}", result as u32));
    }
    Ok(())
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

fn existing_parent(current: &str, default: &Path, workspace: Option<&Path>) -> PathBuf {
    let trimmed = current.trim();
    let mut path = PathBuf::from(trimmed.replace('/', "\\"));
    if !trimmed.is_empty() && path.is_relative() {
        if let Some(workspace) = workspace {
            path = workspace.join(path);
        }
    }
    if path.is_absolute() && !trimmed.contains('\0') {
        let mut ancestors = path.ancestors();
        let _ = ancestors.next();
        if let Some(parent) = ancestors.find(|parent| parent.is_dir()) {
            return parent.to_path_buf();
        }
        if path.is_dir() {
            return path.to_path_buf();
        }
    }
    default.to_path_buf()
}

pub fn select(owner: isize, current: &str, pick: Pick) -> Result<Option<String>, String> {
    check(unsafe { CoInitializeEx(null_mut(), 2) })?;
    let _apartment = Apartment;
    let mut object = null_mut();
    let class = if matches!(pick, Pick::SaveRecovery) { &FILE_SAVE_DIALOG } else { &FILE_OPEN_DIALOG };
    let result = unsafe { CoCreateInstance(class, null_mut(), 1, &FILE_DIALOG, &mut object) };
    let dialog = Interface::from_call(result, object)?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let fallback = exe.parent().ok_or("FolderPickerUnavailable")?;
    let workspace = std::env::var_os("IH_WORKSPACE").map(PathBuf::from);
    let initial = wide(existing_parent(current, fallback, workspace.as_deref()).as_os_str());
    let filter = pick.filter().map(|(name, pattern)| (wide(OsStr::new(name)), wide(OsStr::new(pattern))));
    unsafe {
        let this = dialog.0.as_ptr();
        let get_options: extern "system" fn(*mut c_void, *mut u32) -> i32 = std::mem::transmute(dialog.slot(10));
        let set_options: extern "system" fn(*mut c_void, u32) -> i32 = std::mem::transmute(dialog.slot(9));
        let set_folder: extern "system" fn(*mut c_void, *mut c_void) -> i32 = std::mem::transmute(dialog.slot(12));
        let show: extern "system" fn(*mut c_void, isize) -> i32 = std::mem::transmute(dialog.slot(3));
        let get_result: extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = std::mem::transmute(dialog.slot(20));
        let mut options = 0;
        check(get_options(this, &mut options))?;
        check(set_options(this, options | pick.options()))?;
        if let Some((name, pattern)) = &filter {
            let set_types: extern "system" fn(*mut c_void, u32, *const FileType) -> i32 =
                std::mem::transmute(dialog.slot(4));
            let file_type = FileType { name: name.as_ptr(), pattern: pattern.as_ptr() };
            check(set_types(this, 1, &file_type))?;
        }
        if matches!(pick, Pick::SaveRecovery) {
            let set_extension: extern "system" fn(*mut c_void, *const u16) -> i32 =
                std::mem::transmute(dialog.slot(22));
            let extension = wide(OsStr::new("ihrecovery"));
            check(set_extension(this, extension.as_ptr()))?;
        }
        object = null_mut();
        let result = SHCreateItemFromParsingName(initial.as_ptr(), null(), &SHELL_ITEM, &mut object);
        let start = Interface::from_call(result, object)?;
        check(set_folder(this, start.0.as_ptr()))?;
        match show(this, owner) {
            CANCELLED => return Ok(None),
            result => check(result)?,
        }
        object = null_mut();
        let result = get_result(this, &mut object);
        let item = Interface::from_call(result, object)?;
        let get_name: extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32 = std::mem::transmute(item.slot(5));
        let mut name = null_mut();
        check(get_name(item.0.as_ptr(), FILE_SYSTEM_PATH, &mut name))?;
        if name.is_null() {
            return Err("FolderPickerUnavailable".into());
        }
        let length = (0..32_768).find(|offset| *name.add(*offset) == 0);
        let path = length.map(|length| String::from_utf16(std::slice::from_raw_parts(name, length)));
        CoTaskMemFree(name.cast());
        path.and_then(Result::ok)
            .map(|path| Some(path.replace('\\', "/")))
            .ok_or_else(|| "FolderPickerUnavailable".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_uses_default_and_each_path_resolves_its_own_parent() {
        let exe = std::env::current_exe().unwrap();
        let bin = exe.parent().unwrap();
        let fallback = bin.parent().unwrap();
        assert_eq!(existing_parent("", fallback, Some(bin)), fallback);
        assert_eq!(existing_parent("some-folder", fallback, Some(bin)), bin);
        assert_eq!(
            existing_parent(
                &bin.join("another-folder").to_string_lossy().replace('\\', "/"),
                fallback,
                None,
            ),
            bin,
        );
    }

    #[test]
    fn file_chooser_uses_file_flags_and_type_specific_filters() {
        assert_ne!(Pick::Folder.options() & PICK_FOLDERS, 0);
        for (kind, filter) in [("jar", "*.jar"), ("java", "java.exe"), ("json", "*.json")] {
            let pick = Pick::file(kind).unwrap();
            assert_eq!(pick.options() & PICK_FOLDERS, 0);
            assert_ne!(pick.options() & FILE_MUST_EXIST, 0);
            assert_eq!(pick.filter().unwrap().1, filter);
        }
        assert!(Pick::file("folder").is_err());
        assert_eq!(Pick::SaveRecovery.options() & FILE_MUST_EXIST, 0);
    }
}
