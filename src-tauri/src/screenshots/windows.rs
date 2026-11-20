//! One-shot single-window capture via `Windows.Graphics.Capture` and D3D11.
//!
//! No monitor capture and no process-memory API. All COM objects stay on the
//! calling capture thread and release before `RoUninitialize`.
#![allow(unsafe_code)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{CloseHandle, FILETIME, HMODULE, HWND, LPARAM},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::{
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, IDXGIOutput6,
            },
            Gdi::{HMONITOR, MONITOR_DEFAULTTONULL, MonitorFromWindow},
        },
        System::{
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
                RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
            },
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetDesktopWindow, GetShellWindow, GetWindowDisplayAffinity,
            GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        },
    },
    core::{BOOL, Interface},
};

use super::{Captured, ScreenshotError};

const MAX_PIXELS: u64 = 16_777_216;
const ONCE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Capture the top-level window owned by `pid`.
///
/// # Errors
/// `WindowUnavailable` when the process has no capturable window (closed,
/// minimised, or replaced), `Protected` when the window excludes capture,
/// `Timeout` when no frame arrives, and `Unsupported` on HDR or missing WGC.
pub fn capture_game_window(pid: u32) -> Result<Captured, ScreenshotError> {
    let target = list()?
        .into_iter()
        .find(|candidate| candidate.pid == pid)
        .ok_or(ScreenshotError::WindowUnavailable)?;
    let mut capture = Capture::start(target)?;
    let deadline = std::time::Instant::now() + ONCE_TIMEOUT;
    loop {
        match capture.next()? {
            Some(pixels) => {
                return Ok(Captured {
                    width: pixels.width,
                    height: pixels.height,
                    rgba: pixels.rgba,
                    source: "windows_window",
                });
            }
            None if std::time::Instant::now() >= deadline => return Err(ScreenshotError::Timeout),
            None => std::thread::sleep(std::time::Duration::from_millis(16)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    handle: usize,
    pid: u32,
    created: u64,
    title: String,
}

impl Target {
    fn inspect(handle: usize) -> Result<Self, ScreenshotError> {
        let hwnd = HWND(handle as *mut _);
        // OS metadata only. Process creation time prevents PID reuse.
        unsafe {
            if hwnd == GetDesktopWindow()
                || hwnd == GetShellWindow()
                || !IsWindow(Some(hwnd)).as_bool()
                || !IsWindowVisible(hwnd).as_bool()
                || IsIconic(hwnd).as_bool()
            {
                return Err(ScreenshotError::WindowUnavailable);
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                .map_err(|_| ScreenshotError::WindowUnavailable)?;
            let (mut created, mut exit, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let times = GetProcessTimes(
                process,
                &raw mut created,
                &raw mut exit,
                &raw mut kernel,
                &raw mut user,
            );
            let _ = CloseHandle(process);
            times.map_err(|_| ScreenshotError::WindowUnavailable)?;
            let mut affinity = 0;
            // Inaccessible affinity is not permission to capture.
            GetWindowDisplayAffinity(hwnd, &raw mut affinity)
                .map_err(|_| ScreenshotError::WindowUnavailable)?;
            if affinity != 0 {
                return Err(ScreenshotError::Protected);
            }
            let mut title = [0u16; 512];
            let len = GetWindowTextW(hwnd, &mut title);
            Ok(Self {
                handle,
                pid,
                created: (u64::from(created.dwHighDateTime) << 32)
                    | u64::from(created.dwLowDateTime),
                title: String::from_utf16_lossy(&title[..usize::try_from(len).unwrap_or(0)]),
            })
        }
    }

    fn validate(&self) -> Result<(), ScreenshotError> {
        let actual = Self::inspect(self.handle)?;
        if actual.pid != self.pid || actual.created != self.created {
            return Err(ScreenshotError::WindowUnavailable);
        }
        Ok(())
    }
}

fn list() -> Result<Vec<Target>, ScreenshotError> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // EnumWindows calls synchronously with this live Vec pointer.
        let targets = unsafe { &mut *(data.0 as *mut Vec<Target>) };
        if targets.len() < 100
            && let Ok(target) = Target::inspect(hwnd.0 as usize)
            && target.pid != std::process::id()
            && !target.title.trim().is_empty()
        {
            targets.push(target);
        }
        BOOL(1)
    }
    let mut targets = Vec::<Target>::new();
    unsafe { EnumWindows(Some(collect), LPARAM((&raw mut targets) as isize)) }
        .map_err(|_| ScreenshotError::Unsupported)?;
    Ok(targets)
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct Capture {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    item: GraphicsCaptureItem,
    closed_token: i64,
    closed: Arc<AtomicBool>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    size: SizeInt32,
    target: Target,
    last_time: i64,
    sdr_monitor: HMONITOR,
    // Last field: COM objects release before the apartment is uninitialized.
    _apartment: Apartment,
}

impl Capture {
    fn start(target: Target) -> Result<Self, ScreenshotError> {
        Self::confirm_binding(&target)?;
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|_| ScreenshotError::Unsupported)?;
        let apartment = Apartment;
        check_sdr(HWND(target.handle as *mut _))?;
        Self::create(target, apartment).map_err(|_| ScreenshotError::Unsupported)
    }

    fn confirm_binding(target: &Target) -> Result<(), ScreenshotError> {
        let actual = Target::inspect(target.handle)?;
        if actual.pid != target.pid
            || actual.created != target.created
            || actual.title != target.title
        {
            return Err(ScreenshotError::WindowUnavailable);
        }
        Ok(())
    }

    fn create(target: Target, apartment: Apartment) -> windows::core::Result<Self> {
        if !GraphicsCaptureSession::IsSupported()? {
            return Err(windows::core::Error::empty());
        }
        let hwnd = HWND(target.handle as *mut _);
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        if !valid_size(size) {
            return Err(windows::core::Error::empty());
        }
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )?;
        }
        let device = device.ok_or_else(windows::core::Error::empty)?;
        let context = context.ok_or_else(windows::core::Error::empty)?;
        let dxgi: IDXGIDevice = device.cast()?;
        let direct: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &direct,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(false)?;
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let closed_token = item.Closed(&TypedEventHandler::new(move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(())
        }))?;
        // Border remains enabled: never request borderless capture access.
        session.StartCapture()?;
        Ok(Self {
            session,
            pool,
            item,
            closed_token,
            closed,
            device,
            context,
            size,
            target,
            last_time: -1,
            sdr_monitor: HMONITOR::default(),
            _apartment: apartment,
        })
    }

    fn next(&mut self) -> Result<Option<Pixels>, ScreenshotError> {
        self.target.validate()?;
        unsafe { self.device.GetDeviceRemovedReason() }
            .map_err(|_| ScreenshotError::Unsupported)?;
        if self.item.Size().map_err(|_| ScreenshotError::Unsupported)? != self.size {
            return Err(ScreenshotError::WindowUnavailable);
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(ScreenshotError::WindowUnavailable);
        }
        // Re-check SDR only when the window moves to another monitor.
        let monitor =
            unsafe { MonitorFromWindow(HWND(self.target.handle as *mut _), MONITOR_DEFAULTTONULL) };
        if monitor != self.sdr_monitor {
            check_sdr(HWND(self.target.handle as *mut _))?;
            self.sdr_monitor = monitor;
        }
        let mut newest = None;
        for _ in 0..2 {
            if let Ok(frame) = self.pool.TryGetNextFrame()
                && let Some(old) = newest.replace(frame)
            {
                let _ = old.Close();
            }
        }
        let Some(frame) = newest else {
            return Ok(None);
        };
        let result = (|| {
            let size = frame
                .ContentSize()
                .map_err(|_| ScreenshotError::Unsupported)?;
            if size != self.size || !valid_size(size) {
                return Err(ScreenshotError::WindowUnavailable);
            }
            let time = frame
                .SystemRelativeTime()
                .map_err(|_| ScreenshotError::Unsupported)?
                .Duration;
            if time <= self.last_time {
                return Ok(None);
            }
            self.last_time = time;
            let surface = frame.Surface().map_err(|_| ScreenshotError::Unsupported)?;
            let access: IDirect3DDxgiInterfaceAccess =
                surface.cast().map_err(|_| ScreenshotError::Unsupported)?;
            let texture: ID3D11Texture2D =
                unsafe { access.GetInterface() }.map_err(|_| ScreenshotError::Unsupported)?;
            self.copy(&texture, size)
                .map(Some)
                .map_err(|_| ScreenshotError::Unsupported)
        })();
        let _ = frame.Close();
        result
    }

    fn copy(&self, texture: &ID3D11Texture2D, size: SizeInt32) -> windows::core::Result<Pixels> {
        let width = u32::try_from(size.Width).map_err(|_| windows::core::Error::empty())?;
        let height = u32::try_from(size.Height).map_err(|_| windows::core::Error::empty())?;
        let mut source = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&raw mut source);
        }
        if source.Width < width
            || source.Height < height
            || source.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        {
            return Err(windows::core::Error::empty());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: source.Format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut staging = None;
        unsafe {
            self.device
                .CreateTexture2D(&raw const desc, None, Some(&raw mut staging))?;
        }
        let staging = staging.ok_or_else(windows::core::Error::empty)?;
        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: width,
            bottom: height,
            back: 1,
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context.CopySubresourceRegion(
                &staging,
                0,
                0,
                0,
                0,
                texture,
                0,
                Some(&raw const region),
            );
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))?;
        }
        let row_bytes = width as usize * 4;
        let valid = !mapped.pData.is_null() && mapped.RowPitch as usize >= row_bytes;
        let mut rgba = Vec::new();
        if valid {
            rgba.reserve(row_bytes * height as usize);
            for y in 0..height as usize {
                // Staging texture owns height rows; RowPitch is the documented row stride.
                let row = unsafe {
                    std::slice::from_raw_parts(
                        mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                        row_bytes,
                    )
                };
                for pixel in row.as_chunks::<4>().0 {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
            }
        }
        unsafe {
            self.context.Unmap(&staging, 0);
        }
        if !valid {
            return Err(windows::core::Error::empty());
        }
        Ok(Pixels {
            width,
            height,
            rgba,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
        let _ = self.item.RemoveClosed(self.closed_token);
    }
}

fn valid_size(size: SizeInt32) -> bool {
    size.Width > 0
        && size.Height > 0
        && size.Width <= 8192
        && size.Height <= 8192
        && u64::try_from(size.Width)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(size.Height).unwrap_or(u64::MAX))
            <= MAX_PIXELS
}

fn check_sdr(hwnd: HWND) -> Result<(), ScreenshotError> {
    // Metadata-only output enumeration; never creates a desktop capture.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|_| ScreenshotError::Unsupported)?;
        for a in 0..16 {
            let Ok(adapter) = factory.EnumAdapters1(a) else {
                break;
            };
            for o in 0..16 {
                let Ok(output) = adapter.EnumOutputs(o) else {
                    break;
                };
                let Ok(output): windows::core::Result<IDXGIOutput6> = output.cast() else {
                    continue;
                };
                let Ok(desc) = output.GetDesc1() else {
                    continue;
                };
                if desc.Monitor == monitor {
                    return if desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                        Ok(())
                    } else {
                        Err(ScreenshotError::Unsupported)
                    };
                }
            }
        }
        Err(ScreenshotError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a live Windows desktop shell"]
    fn desktop_shell_is_never_a_capture_target() {
        unsafe {
            assert!(matches!(
                Target::inspect(GetDesktopWindow().0 as usize),
                Err(ScreenshotError::WindowUnavailable)
            ));
            let shell = GetShellWindow();
            assert!(!shell.is_invalid(), "run with an active desktop shell");
            assert!(matches!(
                Target::inspect(shell.0 as usize),
                Err(ScreenshotError::WindowUnavailable)
            ));
        }
    }

    /// Live capture smoke test (ignored by default; needs a desktop GUI session).
    /// Run with `cargo test --lib capture -- --ignored --nocapture`.
    #[test]
    #[ignore = "opens a window and needs a live desktop session"]
    fn a_real_window_produces_a_non_blank_frame() {
        let mut child = std::process::Command::new("notepad.exe")
            .spawn()
            .expect("notepad should start");
        // Give the window time to be created and painted.
        std::thread::sleep(std::time::Duration::from_millis(1200));

        let captured = capture_game_window(child.id());
        let _ = child.kill();
        let _ = child.wait();

        let frame = captured.expect("a visible window must produce a frame");
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(
            frame.rgba.len(),
            (frame.width as usize) * (frame.height as usize) * 4
        );
        // A black frame is what a protected or failed capture looks like, so it
        // must not pass as a successful one.
        let lit = frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] > 8 || px[1] > 8 || px[2] > 8);
        assert!(lit, "the captured frame is blank");
    }
}
