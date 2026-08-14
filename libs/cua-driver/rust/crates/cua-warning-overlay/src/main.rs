use anyhow::Result;
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Observe,
    Active,
    Acting,
    Paused,
    Error,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Observe => "OBSERVE",
            Self::Active => "ACTIVE",
            Self::Acting => "ACTING",
            Self::Paused => "PAUSED",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Parser)]
#[command(name = "cua-warning-overlay")]
struct Args {
    #[arg(long)]
    label: String,
    #[arg(long, default_value = "unknown")]
    session: String,
    #[arg(long, default_value = "Desktop")]
    target: String,
    #[arg(long, value_enum, default_value_t = Mode::Active)]
    mode: Mode,
    #[arg(long)]
    flash: bool,
    #[arg(long)]
    duration: Option<u64>,
}

#[cfg(not(target_os = "windows"))]
fn main() -> Result<()> {
    let args = Args::parse();
    eprintln!(
        "AUTOMATION {}: {} [{}] -> {}",
        args.mode.label(),
        args.label,
        args.session,
        args.target
    );
    std::thread::sleep(std::time::Duration::from_secs(args.duration.unwrap_or(6)));
    Ok(())
}

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    run_windows(Args::parse())
}
#[cfg(target_os = "windows")]
fn run_windows(args: Args) -> Result<()> {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
        InvalidateRect, SelectObject, SetBkMode, SetTextColor, TextOutW, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, GetSystemMetrics, PostQuitMessage, RegisterClassExW,
        SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
        CS_HREDRAW, CS_VREDRAW, HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, MSG, SM_CXVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
        WM_DESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };

    #[derive(Clone)]
    struct State {
        label: String,
        session: String,
        target: String,
        mode: Mode,
        flash: bool,
        phase: bool,
        dpi: u32,
        started: Instant,
        duration: Option<u64>,
    }

    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let dpi = unsafe { GetDpiForSystem() }.max(96);
    STATE
        .set(Mutex::new(State {
            label: args.label,
            session: args.session,
            target: args.target,
            mode: args.mode,
            flash: args.flash,
            phase: true,
            dpi,
            started: Instant::now(),
            duration: args.duration,
        }))
        .ok();

    fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
    }

    fn scaled(px: i32, dpi: u32) -> i32 {
        ((px * dpi as i32) + 95) / 96
    }
    fn background(mode: Mode, phase: bool) -> COLORREF {
        match (mode, phase) {
            (Mode::Observe, true) => rgb(150, 100, 18),
            (Mode::Observe, false) => rgb(64, 43, 8),
            (Mode::Active, true) => rgb(218, 112, 18),
            (Mode::Active, false) => rgb(92, 43, 7),
            (Mode::Acting, true) => rgb(220, 35, 35),
            (Mode::Acting, false) => rgb(95, 5, 5),
            (Mode::Paused, true) => rgb(40, 105, 205),
            (Mode::Paused, false) => rgb(10, 42, 92),
            (Mode::Error, true) => rgb(190, 0, 24),
            (Mode::Error, false) => rgb(78, 0, 10),
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
            WM_TIMER => {
                if let Some(lock) = STATE.get() {
                    let mut s = lock.lock().unwrap();
                    if let Some(limit) = s.duration {
                        if s.started.elapsed().as_secs() >= limit {
                            let _ = DestroyWindow(hwnd);
                            return LRESULT(0);
                        }
                    }
                    if s.flash {
                        s.phase = !s.phase;
                    }
                }
                let _ = InvalidateRect(hwnd, None, true);
                return LRESULT(0);
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = Default::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let (label, detail, mode, phase, dpi) = if let Some(lock) = STATE.get() {
                    let s = lock.lock().unwrap();
                    (
                        format!("AUTOMATION {}  |  {}", s.mode.label(), s.label),
                        format!(
                            "session: {}    target: {}    cua-conductor",
                            s.session, s.target
                        ),
                        s.mode,
                        s.phase,
                        s.dpi,
                    )
                } else {
                    (
                        "AUTOMATION ACTIVE".into(),
                        String::new(),
                        Mode::Active,
                        true,
                        96,
                    )
                };
                let brush = CreateSolidBrush(background(mode, phase));
                FillRect(hdc, &rect, brush);
                let _ = DeleteObject(brush);
                let _ = SetBkMode(hdc, TRANSPARENT);
                let _ = SetTextColor(hdc, rgb(255, 250, 230));

                let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
                let font1 = CreateFontW(
                    -scaled(25, dpi),
                    0,
                    0,
                    0,
                    700,
                    0,
                    0,
                    0,
                    1,
                    0,
                    0,
                    5,
                    0,
                    PCWSTR(face.as_ptr()),
                );
                let old = SelectObject(hdc, font1);
                let line1: Vec<u16> = label.encode_utf16().collect();
                let _ = TextOutW(hdc, scaled(28, dpi), scaled(16, dpi), &line1);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(font1);
                let font2 = CreateFontW(
                    -scaled(14, dpi),
                    0,
                    0,
                    0,
                    500,
                    0,
                    0,
                    0,
                    1,
                    0,
                    0,
                    5,
                    0,
                    PCWSTR(face.as_ptr()),
                );
                let old = SelectObject(hdc, font2);
                let line2: Vec<u16> = detail.encode_utf16().collect();
                let _ = TextOutW(hdc, scaled(30, dpi), scaled(84, dpi), &line2);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(font2);
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, w, l)
    }

    let class_name: Vec<u16> = "Cua.AutomationWarningOverlay\0".encode_utf16().collect();
    let title: Vec<u16> = format!(
        "Cua.AutomationWarningOverlay.{}\0",
        args.mode.label().to_lowercase()
    )
    .encode_utf16()
    .collect();
    let instance = unsafe { GetModuleHandleW(PCWSTR::null())? };
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }

    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = scaled(144, dpi);
    let ex = WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            vx,
            vy,
            vw,
            height,
            None,
            None,
            instance,
            None,
        )?
    };
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 242, LWA_ALPHA)?;
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            vx,
            vy,
            vw,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 1, 500, None);
    }
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_labels_are_stable() {
        assert_eq!(Mode::Observe.label(), "OBSERVE");
        assert_eq!(Mode::Active.label(), "ACTIVE");
        assert_eq!(Mode::Acting.label(), "ACTING");
        assert_eq!(Mode::Paused.label(), "PAUSED");
        assert_eq!(Mode::Error.label(), "ERROR");
    }

    #[test]
    fn cli_parses_acting_mode() {
        let args = Args::try_parse_from([
            "cua-warning-overlay",
            "--label",
            "test",
            "--mode",
            "acting",
            "--flash",
        ])
        .unwrap();
        assert!(matches!(args.mode, Mode::Acting));
        assert!(args.flash);
    }
}
