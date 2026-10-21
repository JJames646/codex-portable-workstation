//! 纯展示层：不参与包校验、目标路径校验和安装任务。
use crate::media::Player;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    UI::{Controls::DRAWITEMSTRUCT, Shell::ShellExecuteW, WindowsAndMessaging::*},
};

pub const WIDTH: i32 = 1000;
pub const HEIGHT: i32 = 650;
const INK: u32 = 0x302012;
const MUTED: u32 = 0x887566;
const BLUE: u32 = 0xff7400;
const WHITE: u32 = 0xffffff;
const PANEL: u32 = 0xfff8f3;
const LOGO: &[u8] = include_bytes!("../assets/logo.bmp");
const SITE: &[u8] = include_bytes!("../assets/official-site.bmp");

pub struct Surface {
    pub parent: HWND,
    pub scale: f32,
    video: HWND,
    page: u8,
    selected: usize,
    last_slide: Instant,
    drag: Option<i32>,
    player: Option<Player>,
    video_attempted: bool,
    video_started: Instant,
}

impl Surface {
    pub fn new() -> Self {
        Self {
            parent: null_mut(),
            scale: 1.0,
            video: null_mut(),
            page: 0,
            selected: 0,
            last_slide: Instant::now(),
            drag: None,
            player: None,
            video_attempted: false,
            video_started: Instant::now(),
        }
    }
    pub fn px(&self, value: i32) -> i32 {
        (value as f32 * self.scale).round() as i32
    }
    pub fn attach(&mut self, parent: HWND) {
        self.parent = parent;
        self.video = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                WS_CHILD,
                0,
                self.px(72),
                self.px(540),
                self.px(530),
                parent,
                null_mut(),
                null_mut(),
                null(),
            )
        };
    }
    pub fn set_page(&mut self, page: u8) {
        if self.page == page {
            return;
        }
        self.page = page;
        if page != 2 {
            self.player = None;
            self.video_attempted = false;
            unsafe {
                ShowWindow(self.video, SW_HIDE);
            }
        }
        self.invalidate();
    }
    pub fn tick(&mut self) {
        if self.page == 2 && !self.video_attempted {
            self.video_attempted = true;
            self.video_started = Instant::now();
            if !self.video.is_null() {
                self.player = Player::open(self.video);
            }
        }
        if let Some(player) = self.player.as_mut() {
            if player.tick() {
                unsafe {
                    ShowWindow(self.video, SW_SHOW);
                }
                player.repaint();
            } else if self.video_started.elapsed() > Duration::from_secs(5) {
                self.player = None;
                unsafe {
                    ShowWindow(self.video, SW_HIDE);
                }
                self.invalidate();
            }
        }
        if self.page < 2
            && self.drag.is_none()
            && self.last_slide.elapsed() > Duration::from_secs(7)
        {
            self.selected = 1 - self.selected;
            self.last_slide = Instant::now();
            self.invalidate();
        }
    }
    pub fn pointer_down(&mut self, x: i32, y: i32) {
        let (x, y) = (x as f32 / self.scale, y as f32 / self.scale);
        if self.page < 2 && (22.0..520.0).contains(&x) && (165.0..470.0).contains(&y) {
            self.drag = Some(x as i32);
        }
    }
    pub fn pointer_up(&mut self, x: i32, y: i32) {
        let (x, y) = (
            (x as f32 / self.scale) as i32,
            (y as f32 / self.scale) as i32,
        );
        if let Some(start) = self.drag.take() {
            if (x - start).abs() > 35 {
                self.selected = 1 - self.selected;
                self.last_slide = Instant::now() + Duration::from_secs(5);
                self.invalidate();
                return;
            }
            if (25..520).contains(&x) && (165..470).contains(&y) {
                self.open_site(self.selected);
                return;
            }
        }
        if y > 602 && x < 300 {
            self.open_site(usize::from(x > 145));
            return;
        }
        if self.page < 2 && (486..551).contains(&y) && (24..518).contains(&x) {
            self.selected = usize::from(x > 270);
            self.last_slide = Instant::now() + Duration::from_secs(5);
            self.invalidate();
        }
        if self.page < 2 && (558..598).contains(&y) && (145..395).contains(&x) {
            self.selected = 1 - self.selected;
            self.last_slide = Instant::now() + Duration::from_secs(5);
            self.invalidate();
        }
    }
    fn open_site(&self, index: usize) {
        let url = if index == 0 {
            "https://go2.jackchen6660.dpdns.org"
        } else {
            "https://go1.jackchen6660.dpdns.org"
        };
        unsafe {
            ShellExecuteW(
                self.parent,
                wide("open").as_ptr(),
                wide(url).as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            );
        }
    }
    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.parent, null(), 0);
        }
    }
    pub fn paint(&self, dc: HDC) {
        fill(dc, self.rect(0, 0, WIDTH, HEIGHT), WHITE);
        fill(dc, self.rect(0, 0, WIDTH, 72), 0xfffbf8);
        bitmap(dc, LOGO, self.rect(24, 12, 48, 48));
        self.text(dc, "云枢AI", 88, 15, 250, 28, 23, 700, INK);
        self.text(dc, "工作站安装程序", 88, 43, 250, 22, 14, 400, MUTED);
        fill(
            dc,
            self.rect(0, 72, 540, 530),
            if self.page == 2 { 0x2c1607 } else { PANEL },
        );
        fill(dc, self.rect(540, 72, 1, 530), 0xeee2d8);
        fill(dc, self.rect(0, 602, WIDTH, 1), 0xeee2d8);
        self.text(dc, "云枢官网 ↗", 26, 618, 115, 24, 14, 500, INK);
        self.text(dc, "│", 144, 618, 18, 24, 14, 400, MUTED);
        self.text(dc, "云枢 API ↗", 170, 618, 125, 24, 14, 500, INK);
        match self.page {
            2 => {
                bitmap(dc, LOGO, self.rect(106, 131, 328, 328));
                self.text(dc, "云枢AI便携工作站", 32, 486, 476, 52, 34, 700, WHITE);
                self.text(
                    dc,
                    "把工作站，随身带走",
                    110,
                    548,
                    350,
                    34,
                    21,
                    400,
                    0xd7c4b0,
                );
            }
            _ => self.paint_carousel(dc),
        }
        let (kicker, title, description) = match self.page {
            0 => (
                "欢迎使用",
                "让工作站，随身出发",
                "完整工作环境，装进一个文件夹。",
            ),
            1 => (
                "准备安装",
                "选择安装位置",
                "将工作站安装到本机磁盘或 U 盘，\n选择一个普通文件夹即可。",
            ),
            2 => (
                "安装进行中",
                "正在安装工作站",
                "请保持设备连接，\n安装完成前请勿拔出 U 盘。",
            ),
            _ => (
                "安装完成",
                "你的工作站，准备好了",
                "全部文件校验完成，即可开始使用。",
            ),
        };
        self.text(dc, kicker, 580, 115, 375, 24, 15, 400, MUTED);
        self.text(dc, title, 580, 151, 400, 47, 31, 700, INK);
        self.text(dc, description, 580, 211, 375, 62, 19, 400, MUTED);
        self.text(
            dc,
            "不格式化磁盘，不删除其他文件。",
            686,
            574,
            290,
            23,
            12,
            400,
            MUTED,
        );
    }
    fn paint_carousel(&self, dc: HDC) {
        self.text(dc, "了解云枢", 28, 91, 300, 24, 15, 400, MUTED);
        self.text(dc, "让 AI 工作，更简单。", 28, 124, 493, 43, 32, 700, INK);
        round(
            dc,
            self.rect(26, 182, 488, 290),
            WHITE,
            0xeeddd2,
            self.px(13),
        );
        if self.selected == 0 {
            bitmap(dc, SITE, self.rect(30, 186, 480, 218));
            self.text(dc, "云枢官网", 43, 418, 275, 26, 20, 600, INK);
            self.text(
                dc,
                "产品介绍 · 下载与使用说明",
                43,
                448,
                360,
                22,
                13,
                400,
                MUTED,
            );
        } else {
            round(
                dc,
                self.rect(30, 186, 480, 218),
                0x30200b,
                0x30200b,
                self.px(10),
            );
            bitmap(dc, LOGO, self.rect(336, 219, 142, 142));
            self.text(dc, "开放的 AI 能力", 54, 230, 290, 48, 30, 700, WHITE);
            self.text(
                dc,
                "简单接入，让强大的模型能力\n融入你的应用。",
                56,
                299,
                280,
                66,
                17,
                400,
                0xeeddd0,
            );
            self.text(dc, "云枢 API", 43, 418, 275, 26, 20, 600, INK);
            self.text(dc, "模型服务与用量", 43, 448, 360, 22, 13, 400, MUTED);
        }
        self.text(dc, "访问网站 ↗", 397, 428, 105, 28, 14, 500, BLUE);
        for (index, x) in [(0, 26), (1, 278)] {
            round(
                dc,
                self.rect(x, 490, 236, 59),
                WHITE,
                if self.selected == index {
                    BLUE
                } else {
                    0xe9dbd0
                },
                self.px(10),
            );
            bitmap(
                dc,
                if index == 0 { SITE } else { LOGO },
                self.rect(x + 10, 500, 68, 38),
            );
            self.text(
                dc,
                if index == 0 {
                    "云枢官网"
                } else {
                    "云枢 API"
                },
                x + 90,
                498,
                141,
                22,
                16,
                600,
                INK,
            );
            self.text(
                dc,
                if index == 0 {
                    "产品与教程"
                } else {
                    "模型服务与用量"
                },
                x + 90,
                526,
                141,
                18,
                12,
                400,
                MUTED,
            );
        }
        self.text(dc, "‹", 166, 561, 30, 30, 27, 400, MUTED);
        self.text(
            dc,
            if self.selected == 0 {
                "01 / 02   ● ○"
            } else {
                "02 / 02   ○ ●"
            },
            218,
            565,
            135,
            24,
            14,
            400,
            MUTED,
        );
        self.text(dc, "›", 373, 561, 30, 30, 27, 400, MUTED);
        self.text(
            dc,
            "自动轮播 · 可左右滑动",
            187,
            588,
            200,
            15,
            10,
            400,
            MUTED,
        );
    }
    pub fn draw_button(&self, draw: &DRAWITEMSTRUCT) {
        let id = draw.CtlID as i32;
        let primary = matches!(id, 1003 | 1007 | 1009);
        let disabled = draw.itemState & 4 != 0;
        let color = if disabled {
            0xeee4dc
        } else if primary {
            BLUE
        } else {
            WHITE
        };
        let border = if primary { color } else { 0xaa9183 };
        round(draw.hDC, draw.rcItem, color, border, self.px(8));
        let mut title = [0u16; 128];
        unsafe {
            GetWindowTextW(draw.hwndItem, title.as_mut_ptr(), 128);
        }
        let title =
            String::from_utf16_lossy(&title[..title.iter().position(|x| *x == 0).unwrap_or(128)]);
        let font = font(self.px(15), 500);
        unsafe {
            let old = SelectObject(draw.hDC, font);
            SetBkMode(draw.hDC, TRANSPARENT as i32);
            SetTextColor(
                draw.hDC,
                if disabled {
                    MUTED
                } else if primary {
                    WHITE
                } else {
                    INK
                },
            );
            let mut rect = draw.rcItem;
            DrawTextW(
                draw.hDC,
                wide(&title).as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if draw.itemState & 16 != 0 {
                InflateRect(&mut rect, -3, -3);
                DrawFocusRect(draw.hDC, &rect);
            }
            SelectObject(draw.hDC, old);
            DeleteObject(font);
        }
    }
    fn text(
        &self,
        dc: HDC,
        value: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        size: i32,
        weight: i32,
        color: u32,
    ) {
        let font = font(self.px(size), weight);
        unsafe {
            let old = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, color);
            let mut rect = self.rect(x, y, w, h);
            DrawTextW(
                dc,
                wide(value).as_ptr(),
                -1,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            DeleteObject(font);
        }
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT {
            left: self.px(x),
            top: self.px(y),
            right: self.px(x + w),
            bottom: self.px(y + h),
        }
    }
}
pub fn font(size: i32, weight: i32) -> *mut c_void {
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Microsoft YaHei UI").as_ptr(),
        )
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn fill(dc: HDC, rect: RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}
fn round(dc: HDC, rect: RECT, color: u32, border: u32, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, border);
        let oldbrush = SelectObject(dc, brush);
        let oldpen = SelectObject(dc, pen);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, oldbrush);
        SelectObject(dc, oldpen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}
fn bitmap(dc: HDC, bytes: &[u8], rect: RECT) {
    if bytes.len() < 54 {
        return;
    }
    let offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    if offset >= bytes.len() {
        return;
    }
    let header =
        unsafe { std::ptr::read_unaligned(bytes[14..].as_ptr().cast::<BITMAPINFOHEADER>()) };
    let info = BITMAPINFO {
        bmiHeader: header,
        bmiColors: [RGBQUAD::default()],
    };
    let scale = ((rect.right - rect.left) as f32 / header.biWidth as f32)
        .min((rect.bottom - rect.top) as f32 / header.biHeight.abs() as f32);
    let width = (header.biWidth as f32 * scale).round() as i32;
    let height = (header.biHeight.abs() as f32 * scale).round() as i32;
    unsafe {
        SetStretchBltMode(dc, HALFTONE);
        StretchDIBits(
            dc,
            rect.left + (rect.right - rect.left - width) / 2,
            rect.top + (rect.bottom - rect.top - height) / 2,
            width,
            height,
            0,
            0,
            header.biWidth,
            header.biHeight.abs(),
            bytes[offset..].as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}
