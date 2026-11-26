//! Optional separate click-through shadow for a layered fence window.
//! It is owned by the fence so it follows the same desktop Z-order group.
use aura_windows::gfx::Surface;
use windows::{core::w,Win32::{
    Foundation::{HANDLE,HWND,LPARAM,LRESULT,POINT,SIZE,WPARAM},
    Graphics::Gdi::{GetDC,ReleaseDC,AC_SRC_ALPHA,AC_SRC_OVER,BLENDFUNCTION},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
}};
pub(super) const CLASS_NAME:&str="AuraFences.Shadow";
const CLASS:windows::core::PCWSTR=w!("AuraFences.Shadow");
const PROPERTY:windows::core::PCWSTR=w!("AuraFences.ShadowWindow");
// Avoid unexpectedly allocating large bitmaps on extreme multi-monitor desktops.
const MAX_BYTES:usize=64*1024*1024;
unsafe extern "system" fn proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT {
    match msg {
        WM_NCHITTEST=>LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE=>LRESULT(MA_NOACTIVATE as isize),
        _=>unsafe{DefWindowProcW(hwnd,msg,wp,lp)},
    }
}
fn register()->bool {
    static REGISTERED:std::sync::OnceLock<bool>=std::sync::OnceLock::new();
    *REGISTERED.get_or_init(||unsafe{
        let Ok(instance)=GetModuleHandleW(None) else{return false};
        RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(proc),hInstance:instance.into(),lpszClassName:CLASS,..Default::default()})!=0
    })
}
fn margin(dpi:u32)->i32 {((18u64*u64::from(dpi.max(96))+48)/96).min(256) as i32}

/// Called directly by the fence procedure, without borrowing DesktopHost. The
/// owner can be moved/shown while the host is already mutably borrowed.
pub fn sync(owner:HWND,visible:Option<bool>) {
    let child=HWND(unsafe{GetPropW(owner,PROPERTY)}.0);
    if child.0.is_null() || !unsafe{IsWindow(Some(child))}.as_bool(){return;}
    if visible==Some(false) {if unsafe{IsWindowVisible(child)}.as_bool(){unsafe{let _=ShowWindow(child,SW_HIDE);}}return;}
    let mut rect=windows::Win32::Foundation::RECT::default();
    if unsafe{GetWindowRect(owner,&mut rect)}.is_err(){return;}
    let dpi=unsafe{windows::Win32::UI::HiDpi::GetDpiForWindow(owner)};
    let m=margin(dpi);
    let mut flags=SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE;
    if visible==Some(true) && !unsafe{IsWindowVisible(child)}.as_bool(){flags|=SWP_SHOWWINDOW;}
    let _=unsafe{SetWindowPos(child,None,rect.left-m,rect.top-m,0,0,flags)};
}

pub struct Shadow { hwnd:HWND,owner:HWND,key:Option<(i32,i32,u32,u8,u8)> }
impl Shadow {
    #[cfg(test)] pub(super) fn hwnd(&self)->HWND {self.hwnd}
    pub fn new(owner:HWND)->Option<Self>{
        if !register(){return None;}
        let instance=unsafe{GetModuleHandleW(None)}.ok()?;
        let hwnd=unsafe{CreateWindowExW(WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW,CLASS,w!(""),WS_POPUP,
            0,0,1,1,Some(owner),None,Some(instance.into()),None)}.ok()?;
        if unsafe{SetPropW(owner,PROPERTY,Some(HANDLE(hwnd.0)))}.is_err(){let _=unsafe{DestroyWindow(hwnd)};return None;}
        Some(Self{hwnd,owner,key:None})
    }
    pub fn update(&mut self,dpi:u32,radius:u8,strength:u8)->bool {
        if !unsafe{IsWindow(Some(self.owner))}.as_bool() || !unsafe{IsWindow(Some(self.hwnd))}.as_bool(){return false;}
        let mut rect=windows::Win32::Foundation::RECT::default();if unsafe{GetWindowRect(self.owner,&mut rect)}.is_err(){return false;}
        let (w,h)=(rect.right-rect.left,rect.bottom-rect.top);if w<=0||h<=0{return false;}
        let key=(w,h,dpi,radius,strength);
        if self.key!=Some(key) {
            let m=margin(dpi);let Some(tw)=w.checked_add(m*2)else{return false};let Some(th)=h.checked_add(m*2)else{return false};
            let Some(len)=(tw as usize).checked_mul(th as usize).and_then(|n|n.checked_mul(4)).filter(|n|*n<=MAX_BYTES)else{unsafe{let _=ShowWindow(self.hwnd,SW_HIDE);}self.key=None;return false};
            let Some(surface)=Surface::new(tw,th)else{unsafe{let _=ShowWindow(self.hwnd,SW_HIDE);}self.key=None;return false};
            if surface.bits.is_null(){return false;}
            let pixels=unsafe{std::slice::from_raw_parts_mut(surface.bits,len)};
            let factor=dpi.max(96) as f32/96.0;
            if !super::shadow_mask::paint(pixels,w as usize,h as usize,m as usize,f32::from(radius)*factor,3.0*factor,strength){return false;}
            let screen=unsafe{GetDC(None)};let position=POINT{x:rect.left-m,y:rect.top-m};let size=SIZE{cx:tw,cy:th};let origin=POINT::default();
            let blend=BLENDFUNCTION{BlendOp:AC_SRC_OVER as u8,BlendFlags:0,SourceConstantAlpha:255,AlphaFormat:AC_SRC_ALPHA as u8};
            let result=unsafe{UpdateLayeredWindow(self.hwnd,Some(screen),Some(&position),Some(&size),Some(surface.dc),Some(&origin),Default::default(),Some(&blend),ULW_ALPHA)};
            unsafe{ReleaseDC(None,screen);}
            if result.is_err(){let _=unsafe{ShowWindow(self.hwnd,SW_HIDE)};self.key=None;return false;}
            self.key=Some(key);
        }
        if unsafe{IsWindowVisible(self.owner)}.as_bool(){sync(self.owner,Some(true));}
        true
    }
}
impl Drop for Shadow {
    fn drop(&mut self){unsafe{
        if IsWindow(Some(self.owner)).as_bool() && GetPropW(self.owner,PROPERTY).0==self.hwnd.0 {let _=RemovePropW(self.owner,PROPERTY);}
        if IsWindow(Some(self.hwnd)).as_bool(){let _=DestroyWindow(self.hwnd);}
    }}
}
