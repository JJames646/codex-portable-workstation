//! 安装时才加载系统 MFPlay；缺失媒体组件不会阻止安装器启动。
use std::{ffi::c_void, fs, path::PathBuf, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE, HWND},
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
    },
};

pub struct Player {
    object: *mut c_void,
    library: HMODULE,
    path: PathBuf,
    com: bool,
    played: bool,
    restarting: bool,
}

type Create =
    unsafe extern "system" fn(*const u16, i32, u32, *mut c_void, HWND, *mut *mut c_void) -> i32;
type Simple = unsafe extern "system" fn(*mut c_void) -> i32;

#[link(name = "ole32")]
unsafe extern "system" {
    fn PropVariantClear(value: *mut c_void) -> i32;
}

impl Player {
    pub fn open(hwnd: HWND) -> Option<Self> {
        let library = unsafe {
            LoadLibraryExW(
                wide("mfplay.dll").as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if library.is_null() {
            return None;
        }
        let mut player = Self {
            object: null_mut(),
            library,
            path: PathBuf::new(),
            com: false,
            played: false,
            restarting: false,
        };
        let function = unsafe { GetProcAddress(library, c"MFPCreateMediaPlayer".as_ptr().cast()) }?;
        let create: Create = unsafe { std::mem::transmute(function) };
        player.com = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED as u32) } >= 0;
        player.path = std::env::temp_dir().join(format!(
            "yunshu-installer-video-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        // 素材已在构建时移除音轨，播放器异步初始化也不会瞬间发声。
        fs::write(&player.path, include_bytes!("../assets/promo-fit-silent.mp4")).ok()?;
        let result = unsafe {
            create(
                wide(&player.path.to_string_lossy()).as_ptr(),
                1,
                0,
                null_mut(),
                hwnd,
                &mut player.object,
            )
        };
        if result < 0 || player.object.is_null() {
            return None;
        }
        Some(player)
    }

    unsafe fn method<T: Copy>(&self, index: usize) -> T {
        let table = unsafe { *(self.object as *const *const *const c_void) };
        unsafe { std::mem::transmute_copy(&*table.add(index)) }
    }

    pub fn tick(&mut self) -> bool {
        // IMFPMediaPlayer 槽位依据当前 Windows SDK mfplay.h，IUnknown 占前三项。
        let state_fn: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 =
            unsafe { self.method(13) };
        let mut state = 0;
        if unsafe { state_fn(self.object, &mut state) } < 0 {
            return false;
        }
        if state == 2 {
            self.played = true;
        }
        // EOF 不保证变为 STOPPED；以实际媒体位置判定循环，不用墙上时钟猜进度。
        let at_end = self.played
            && self
                .time_value(8)
                .zip(self.time_value(9))
                .is_some_and(|(position, duration)| duration > 0 && position >= duration - 100_000);
        if at_end && !self.restarting {
            let stop: Simple = unsafe { self.method(5) };
            self.restarting = unsafe { stop(self.object) } >= 0;
        } else if state == 1 && self.played {
            let play: Simple = unsafe { self.method(3) };
            self.restarting = false;
            unsafe {
                play(self.object);
            }
        }
        self.played && state != 4
    }

    fn time_value(&self, index: usize) -> Option<i64> {
        // x64 PROPVARIANT 为 24 字节，VT_I8 的数值位于偏移 8；SDK 的 100ns GUID 全零。
        let get: unsafe extern "system" fn(*mut c_void, *const u32, *mut u64) -> i32 =
            unsafe { self.method(index) };
        let mut value = [0_u64; 3];
        let result = unsafe { get(self.object, [0_u32; 4].as_ptr(), value.as_mut_ptr()) };
        let time = (result >= 0 && value[0] as u16 == 20).then_some(value[1] as i64);
        unsafe {
            PropVariantClear(value.as_mut_ptr().cast());
        }
        time
    }

    pub fn repaint(&self) {
        let update: Simple = unsafe { self.method(32) };
        unsafe {
            update(self.object);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if !self.object.is_null() {
            let shutdown: Simple = unsafe { self.method(38) };
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { self.method(2) };
            unsafe {
                shutdown(self.object);
                release(self.object);
            }
        }
        unsafe {
            if self.com {
                CoUninitialize();
            }
            FreeLibrary(self.library);
        }
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
