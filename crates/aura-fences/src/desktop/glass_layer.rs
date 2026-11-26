//! Composition-backed, rounded glass behind a layered fence.
//! The legacy accent policy blurs the rectangular HWND even when SetWindowRgn
//! or the layered bitmap has transparent corners. A dedicated no-redirection
//! window clips its host-backdrop visual with the same rounded geometry.
use std::{cell::RefCell, mem::size_of};
use windows::{
    core::{w, Interface},
    System::DispatcherQueueController,
    Win32::{
        Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH},
        System::{
            LibraryLoader::GetModuleHandleW,
            WinRT::Composition::ICompositorDesktopInterop,
            WinRT::{
                CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE,
                DQTYPE_THREAD_CURRENT,
            },
        },
        UI::WindowsAndMessaging::*,
    },
    UI::Composition::{
        CompositionRoundedRectangleGeometry, Compositor, Desktop::DesktopWindowTarget, SpriteVisual,
    },
};
use windows_numerics::Vector2;

pub(super) const CLASS_NAME: &str = "AuraFences.GlassLayer";
const CLASS: windows::core::PCWSTR = w!("AuraFences.GlassLayer");
const PROPERTY: windows::core::PCWSTR = w!("AuraFences.GlassLayerWindow");

struct Context {
    compositor: Compositor,
    _queue: Option<DispatcherQueueController>,
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
fn compositor() -> Option<Compositor> {
    CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let queue = if windows::System::DispatcherQueue::GetForCurrentThread().is_ok() {
                None
            } else {
                Some(
                    unsafe {
                        CreateDispatcherQueueController(DispatcherQueueOptions {
                            dwSize: size_of::<DispatcherQueueOptions>() as u32,
                            threadType: DQTYPE_THREAD_CURRENT,
                            apartmentType: DQTAT_COM_NONE,
                        })
                    }
                    .ok()?,
                )
            };
            *slot = Some(Context {
                compositor: Compositor::new().ok()?,
                _queue: queue,
            });
        }
        slot.as_ref().map(|c| c.compositor.clone())
    })
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
fn register() -> bool {
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REGISTERED.get_or_init(|| unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return false;
        };
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        }) != 0
    })
}
struct VisualState {
    geometry: CompositionRoundedRectangleGeometry,
    visual: SpriteVisual,
    radius: f32,
}
impl VisualState {
    fn resize(&self, width: i32, height: i32) -> windows::core::Result<()> {
        let size = Vector2 {
            X: width.max(1) as f32,
            Y: height.max(1) as f32,
        };
        let r = self.radius.min(size.X.min(size.Y) / 2.0).max(0.0);
        self.visual.SetSize(size)?;
        self.geometry.SetSize(size)?;
        self.geometry.SetCornerRadius(Vector2 { X: r, Y: r })?;
        Ok(())
    }
}
/// Synchronous move/resize/show notifications can arrive while the host is borrowed.
/// The HWND property avoids borrowing the host again in its window procedure.
pub fn sync(owner: HWND, visible: Option<bool>) {
    let layer = HWND(unsafe { GetPropW(owner, PROPERTY) }.0);
    if layer.0.is_null() || !unsafe { IsWindow(Some(layer)) }.as_bool() {
        return;
    }
    if visible == Some(false) {
        if unsafe { IsWindowVisible(layer) }.as_bool() {
            let _ = unsafe { ShowWindow(layer, SW_HIDE) };
        }
        return;
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(owner, &mut rect) }.is_err() {
        return;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let raw = unsafe { GetWindowLongPtrW(layer, GWLP_USERDATA) } as *const VisualState;
    if !raw.is_null() {
        let _ = unsafe { (&*raw).resize(width, height) };
    }
    let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING;
    if visible == Some(true) {
        flags |= SWP_SHOWWINDOW;
    }
    // Insert directly behind the fence; both windows have Progman as owner.
    let _ = unsafe {
        SetWindowPos(
            layer,
            Some(owner),
            rect.left,
            rect.top,
            width,
            height,
            flags,
        )
    };
}

pub struct Glass {
    owner: HWND,
    hwnd: HWND,
    state: Box<VisualState>,
    _target: DesktopWindowTarget,
    compositor: Compositor,
    blur_style: Option<(bool, u32)>,
}
impl Glass {
    pub fn new(owner: HWND) -> Option<Self> {
        if !register() {
            return None;
        }
        let c = compositor()?;
        let instance = unsafe { GetModuleHandleW(None) }.ok()?;
        let desktop = unsafe { GetWindow(owner, GW_OWNER) }.ok();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                desktop,
                None,
                Some(instance.into()),
                None,
            )
        }
        .ok()?;
        let build = || -> windows::core::Result<(Box<VisualState>, DesktopWindowTarget)> {
            let enabled = 1i32;
            unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_HOSTBACKDROPBRUSH,
                    (&enabled as *const i32).cast(),
                    size_of::<i32>() as u32,
                )
            }?;
            let target = unsafe {
                c.cast::<ICompositorDesktopInterop>()?
                    .CreateDesktopWindowTarget(hwnd, true)
            }?;
            let visual = c.CreateSpriteVisual()?;
            visual.SetBrush(&c.CreateHostBackdropBrush()?)?;
            let geometry = c.CreateRoundedRectangleGeometry()?;
            visual.SetClip(&c.CreateGeometricClipWithGeometry(&geometry)?)?;
            target.SetRoot(&visual)?;
            Ok((
                Box::new(VisualState {
                    geometry,
                    visual,
                    radius: 0.0,
                }),
                target,
            ))
        };
        let (mut state, target) = match build() {
            Ok(v) => v,
            Err(_) => {
                let _ = unsafe { DestroyWindow(hwnd) };
                return None;
            }
        };
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&mut *state as *mut VisualState) as isize,
            );
        }
        if unsafe { SetPropW(owner, PROPERTY, Some(HANDLE(hwnd.0))) }.is_err() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }
        Some(Self {
            owner,
            hwnd,
            state,
            _target: target,
            compositor: c,
            blur_style: None,
        })
    }
    pub fn update(&mut self, dpi: u32, radius: u8, extra_blur: bool) -> bool {
        if !unsafe { IsWindow(Some(self.owner)) }.as_bool()
            || !unsafe { IsWindow(Some(self.hwnd)) }.as_bool()
        {
            return false;
        }
        let key = (extra_blur, dpi.max(96));
        if self.blur_style != Some(key) {
            let brush = self
                .compositor
                .CreateHostBackdropBrush()
                .and_then(|b| b.cast());
            if let Ok(brush) = brush {
                // A host-backdrop brush only samples pixels; it is not itself a
                // blur. Always apply Gaussian blur, with a stronger optional setting.
                let amount = if extra_blur { 16.0 } else { 8.0 };
                let Ok(composed) = super::glass_effects::blurred_backdrop(
                    &self.compositor,
                    &brush,
                    amount * dpi.max(96) as f32 / 96.0,
                ) else {
                    return false;
                };
                if self.state.visual.SetBrush(&composed).is_err() {
                    return false;
                }
                self.blur_style = Some(key);
            } else {
                return false;
            }
        }
        self.state.radius = f32::from(radius) * dpi.max(96) as f32 / 96.0;
        sync(
            self.owner,
            Some(unsafe { IsWindowVisible(self.owner) }.as_bool()),
        );
        true
    }
}
impl Drop for Glass {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.owner)).as_bool()
                && GetPropW(self.owner, PROPERTY).0 == self.hwnd.0
            {
                let _ = RemovePropW(self.owner, PROPERTY);
            }
            if IsWindow(Some(self.hwnd)).as_bool() {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
