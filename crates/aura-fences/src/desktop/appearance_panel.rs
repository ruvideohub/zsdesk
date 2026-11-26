//! Fences appearance settings using standard Windows controls. Native trackbars
//! expose their names, values and ranges to accessibility tools and honor high
//! contrast and keyboard settings without a dependency on Dock.
use std::cell::Cell;
use aura_fences_model::{appearance::Control, desktop::{DesktopSettings,FenceTheme}};
use windows::{core::{w,PCWSTR}, Win32::{
    Foundation::{COLORREF,HWND,LPARAM,LRESULT,POINT,WPARAM},
    Graphics::Gdi::{CreateSolidBrush,DeleteObject,FillRect,GetMonitorInfoW,MonitorFromPoint,GetStockObject,SetBkColor,SetTextColor,SetBkMode,HBRUSH,HDC,TRANSPARENT,DEFAULT_GUI_FONT,COLOR_WINDOW,MONITORINFO,MONITOR_DEFAULTTONEAREST},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::{InitCommonControlsEx,INITCOMMONCONTROLSEX,ICC_BAR_CLASSES,TRACKBAR_CLASSW,TBS_HORZ,TBS_NOTICKS,TBM_SETPOS,TBM_SETRANGEMIN,TBM_SETRANGEMAX},
        HiDpi::{GetDpiForMonitor,MDT_EFFECTIVE_DPI}, Input::KeyboardAndMouse::{EnableWindow,SetFocus}, WindowsAndMessaging::*},
}};
const CLASS:PCWSTR=w!("AuraFences.AppearanceSettings");
const TBM_GETPOS:u32=WM_USER;
const CHANGE:u32=WM_APP+705;
const FINISH:u32=WM_APP+706;
const BLUR_ID:usize=200;
const DONE_ID:usize=201;
const ACRYLIC_ID:usize=202;
const THEME_ID:usize=203;
const COUNTS_ID:usize=204;
thread_local!{
    static OPEN:Cell<bool>=const{Cell::new(false)};
    // Native controls remain keyboard/UIA accessible. In high contrast the
    // system owns all colors; otherwise the panel uses the shared Dock tokens.
    static PANEL_BRUSH:Cell<isize>=const{Cell::new(0)};
    static PANEL_FG:Cell<u32>=const{Cell::new(0)};
    static PANEL_BG:Cell<u32>=const{Cell::new(0)};
}
fn rgb(r:u8,g:u8,b:u8)->COLORREF { COLORREF(u32::from(r) | u32::from(g)<<8 | u32::from(b)<<16) }
struct PaletteGuard;
impl PaletteGuard {
    fn new(dark:bool)->Self {
        if !aura_windows::theme::high_contrast() {
            let [r,g,b]=if dark {aura_core::desktop_style::SURFACE_DARK} else {aura_core::desktop_style::SURFACE_LIGHT};
            let bg=rgb(r,g,b); let fg=if dark {rgb(242,242,246)} else {rgb(27,27,32)};
            let brush=unsafe{CreateSolidBrush(bg)};
            PANEL_BRUSH.with(|v|v.set(brush.0 as isize));PANEL_BG.with(|v|v.set(bg.0));PANEL_FG.with(|v|v.set(fg.0));
        }
        Self
    }
}
impl Drop for PaletteGuard { fn drop(&mut self) { let brush=PANEL_BRUSH.with(|v|v.replace(0)); if brush!=0 {let _=unsafe{DeleteObject(HBRUSH(brush as *mut _).into())};} } }

unsafe extern "system" fn proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT {
    match msg {
        WM_ERASEBKGND if PANEL_BRUSH.with(|v|v.get())!=0 => {
            let mut r=windows::Win32::Foundation::RECT::default();
            if unsafe{GetClientRect(hwnd,&mut r)}.is_ok(){unsafe{FillRect(HDC(wp.0 as *mut _),&r,HBRUSH(PANEL_BRUSH.with(|v|v.get()) as *mut _));}}
            LRESULT(1)
        }
        WM_CTLCOLORSTATIC|WM_CTLCOLORBTN if PANEL_BRUSH.with(|v|v.get())!=0 => {
            let dc=HDC(wp.0 as *mut _);
            unsafe{SetTextColor(dc,COLORREF(PANEL_FG.with(|v|v.get())));SetBkColor(dc,COLORREF(PANEL_BG.with(|v|v.get())));SetBkMode(dc,TRANSPARENT);}
            LRESULT(PANEL_BRUSH.with(|v|v.get()))
        }
        WM_HSCROLL => {let _=unsafe{PostMessageW(Some(hwnd),CHANGE,WPARAM(0),lp)};LRESULT(0)}
        WM_COMMAND if wp.0 & 0xffff == BLUR_ID || wp.0 & 0xffff == ACRYLIC_ID || wp.0 & 0xffff == COUNTS_ID || (wp.0 & 0xffff == THEME_ID && (wp.0 >> 16) as u16 == CBN_SELCHANGE as u16) => {let _=unsafe{PostMessageW(Some(hwnd),CHANGE,WPARAM(wp.0 & 0xffff),LPARAM(0))};LRESULT(0)}
        WM_COMMAND if wp.0 & 0xffff == DONE_ID => {let _=unsafe{PostMessageW(Some(hwnd),FINISH,WPARAM(0),LPARAM(0))};LRESULT(0)}
        WM_CLOSE|WM_DESTROY => {let _=unsafe{PostMessageW(Some(hwnd),FINISH,WPARAM(0),LPARAM(0))};LRESULT(0)}
        _=>unsafe{DefWindowProcW(hwnd,msg,wp,lp)},
    }
}
fn register()->bool {
    static REGISTERED:std::sync::OnceLock<bool>=std::sync::OnceLock::new();
    *REGISTERED.get_or_init(||unsafe{
        if !InitCommonControlsEx(&INITCOMMONCONTROLSEX{dwSize:std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,dwICC:ICC_BAR_CLASSES}).as_bool(){return false;}
        let Ok(instance)=GetModuleHandleW(None) else{return false};
        RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(proc),hInstance:instance.into(),lpszClassName:CLASS,
            hCursor:LoadCursorW(None,IDC_ARROW).unwrap_or_default(),hbrBackground:windows::Win32::Graphics::Gdi::HBRUSH((COLOR_WINDOW.0+1) as usize as *mut _),..Default::default()})!=0
    })
}
struct Window(HWND);
impl Drop for Window {fn drop(&mut self){unsafe{if IsWindow(Some(self.0)).as_bool(){let _=DestroyWindow(self.0);}}}}
struct OpenGuard;
impl Drop for OpenGuard {fn drop(&mut self){OPEN.with(|v|v.set(false));}}
fn wide(text:&str)->Vec<u16>{text.encode_utf16().chain(Some(0)).collect()}
fn text(key:&str)->Vec<u16>{wide(aura_core::i18n::text(key))}

/// A small owned settings window, with immediate application and debounced save.
/// Closing or Escape keeps changes, just like the other Fences menus.
pub(super) fn show(owner:HWND,point:POINT,mut value:DesktopSettings,dark:bool,mut apply:impl FnMut(&DesktopSettings)) {
    if OPEN.with(|v|v.replace(true)){return;}let _open=OpenGuard;
    if !register(){return;}
    let _palette=PaletteGuard::new(dark);
    let monitor=unsafe{MonitorFromPoint(point,MONITOR_DEFAULTTONEAREST)};
    let mut info=MONITORINFO{cbSize:std::mem::size_of::<MONITORINFO>() as u32,..Default::default()};
    if !unsafe{GetMonitorInfoW(monitor,&mut info)}.as_bool(){return;}
    let(mut dx,mut dy)=(96,96);
    let dpi=if unsafe{GetDpiForMonitor(monitor,MDT_EFFECTIVE_DPI,&mut dx,&mut dy)}.is_ok(){dx.max(96)}else{96};
    let work=info.rcWork;let scale=(dpi as f32/96.0).min((work.right-work.left) as f32/520.0).min((work.bottom-work.top) as f32/670.0).max(0.5);
    let px=|x:i32|(x as f32*scale).round() as i32;
    let (width,height)=(px(500),px(650));
    let left=(point.x-width/2).clamp(work.left,(work.right-width).max(work.left));
    let top=(point.y-height/2).clamp(work.top,(work.bottom-height).max(work.top));
    let Ok(instance)=(unsafe{GetModuleHandleW(None)}) else{return};
    let title=text("格子外观设置");
    let Ok(hwnd)=(unsafe{CreateWindowExW(WS_EX_CONTROLPARENT,CLASS,PCWSTR(title.as_ptr()),WS_POPUP|WS_CAPTION|WS_SYSMENU|WS_CLIPCHILDREN,left,top,width,height,Some(owner),None,Some(instance.into()),None)})else{return};
    let _window=Window(hwnd);
    aura_windows::theme::set_dark_frame(hwnd,dark);
    aura_windows::theme::round_corners(hwnd,false);
    let child=|class:PCWSTR,title:&[u16],style:WINDOW_STYLE,id:usize,x:i32,y:i32,w:i32,h:i32|->Option<HWND>{
        let result=unsafe{CreateWindowExW(WINDOW_EX_STYLE(0),class,PCWSTR(title.as_ptr()),WS_CHILD|WS_VISIBLE|style,px(x),px(y),px(w),px(h),Some(hwnd),Some(HMENU(id as *mut _)),Some(instance.into()),None)}.ok()?;
        aura_windows::theme::theme_control(result,dark);
        let font=unsafe{GetStockObject(DEFAULT_GUI_FONT)};
        unsafe{SendMessageW(result,WM_SETFONT,Some(WPARAM(font.0 as usize)),Some(LPARAM(1)));}
        Some(result)
    };
    let caption=text("更改立即生效");
    if child(w!("STATIC"),&caption,WINDOW_STYLE(0),0,20,12,450,22).is_none(){return;}
    let theme_label=text("外观主题");
    if child(w!("STATIC"),&theme_label,WINDOW_STYLE(0),0,20,42,190,24).is_none(){return;}
    let empty=wide("");
    let Some(theme_box)=child(w!("COMBOBOX"),&empty,WS_TABSTOP|WINDOW_STYLE((CBS_DROPDOWNLIST|WS_VSCROLL.0 as i32) as u32),THEME_ID,210,38,255,140) else{return};
    for key in ["跟随系统","深色","浅色"] {
        let option=text(key);unsafe{SendMessageW(theme_box,CB_ADDSTRING,Some(WPARAM(0)),Some(LPARAM(option.as_ptr() as isize)));}
    }
    let selection=match value.theme {FenceTheme::System=>0,FenceTheme::Dark=>1,FenceTheme::Light=>2};
    unsafe{SendMessageW(theme_box,CB_SETCURSEL,Some(WPARAM(selection)),None);}
    let mut sliders=Vec::new();
    for (i,control) in Control::ALL[..7].iter().copied().enumerate(){
        let y=82+i as i32*53;let label=text(control.label());
        if child(w!("STATIC"),&label,WINDOW_STYLE(0),0,20,y+4,190,24).is_none(){return;}
        let Some(track)=child(TRACKBAR_CLASSW,&label,WS_TABSTOP|WINDOW_STYLE((TBS_HORZ|TBS_NOTICKS) as u32),100+i,210,y,200,32) else{return};
        let (min,max)=control.range();
        unsafe{SendMessageW(track,TBM_SETRANGEMIN,Some(WPARAM(0)),Some(LPARAM(min as isize)));
            SendMessageW(track,TBM_SETRANGEMAX,Some(WPARAM(0)),Some(LPARAM(max as isize)));
            SendMessageW(track,TBM_SETPOS,Some(WPARAM(1)),Some(LPARAM(control.get(&value) as isize)));}
        let display=wide(&format!("{}{}",control.get(&value),control.unit()));
        let Some(number)=child(w!("STATIC"),&display,WINDOW_STYLE(0),0,420,y+4,62,24) else{return};
        sliders.push((control,track,number));
    }
    let blur=text(Control::Blur.label());
    let Some(checkbox)=child(w!("BUTTON"),&blur,WS_TABSTOP|WINDOW_STYLE(BS_AUTOCHECKBOX as u32),BLUR_ID,20,458,320,28) else{return};
    unsafe{SendMessageW(checkbox,BM_SETCHECK,Some(WPARAM(value.blur as usize)),None);}
    let acrylic=text("额外背景模糊");
    let Some(acrylic_box)=child(w!("BUTTON"),&acrylic,WS_TABSTOP|WINDOW_STYLE(BS_AUTOCHECKBOX as u32),ACRYLIC_ID,20,493,430,28) else{return};
    unsafe{SendMessageW(acrylic_box,BM_SETCHECK,Some(WPARAM(value.acrylic_blur as usize)),None);let _=EnableWindow(acrylic_box,value.blur);}
    let counts=text("显示文件数量");
    let Some(counts_box)=child(w!("BUTTON"),&counts,WS_TABSTOP|WINDOW_STYLE(BS_AUTOCHECKBOX as u32),COUNTS_ID,20,528,430,28) else{return};
    unsafe{SendMessageW(counts_box,BM_SETCHECK,Some(WPARAM(value.show_counts as usize)),None);}
    let done=text("完成");
    if child(w!("BUTTON"),&done,WS_TABSTOP|WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),DONE_ID,355,566,110,30).is_none(){return;}
    unsafe{let _=ShowWindow(hwnd,SW_SHOWNORMAL);let _=SetForegroundWindow(hwnd);let _=SetFocus(Some(sliders[0].1));}
    loop {
        let mut msg=MSG::default();let result=unsafe{GetMessageW(&mut msg,None,0,0)}.0;
        if result==0{unsafe{PostQuitMessage(msg.wParam.0 as i32)};break;}if result<0{break;}
        if msg.hwnd==hwnd && msg.message==FINISH{break;}
        if msg.hwnd==hwnd && msg.message==CHANGE {
            if msg.wParam.0==THEME_ID {
                let selection=unsafe{SendMessageW(theme_box,CB_GETCURSEL,None,None)}.0;
                let next=match selection {0=>FenceTheme::System,1=>FenceTheme::Dark,2=>FenceTheme::Light,_=>value.theme};
                if next!=value.theme {value.theme=next;apply(&value);}
            } else if msg.wParam.0==COUNTS_ID {
                let next=unsafe{SendMessageW(counts_box,BM_GETCHECK,None,None)}.0!=0;
                if next!=value.show_counts {value.show_counts=next;apply(&value);}
            } else if msg.wParam.0==BLUR_ID || msg.wParam.0==ACRYLIC_ID {
                let (control,box_hwnd)=if msg.wParam.0==BLUR_ID {(Control::Blur,checkbox)} else {(Control::Acrylic,acrylic_box)};
                let checked=unsafe{SendMessageW(box_hwnd,BM_GETCHECK,None,None)}.0!=0;
                if i32::from(checked)!=control.get(&value) {control.set(&mut value,i32::from(checked));if msg.wParam.0==BLUR_ID {let _=unsafe{EnableWindow(acrylic_box,checked)};}apply(&value);}
            } else if let Some((control,track,number))=sliders.iter().find(|(_,track,_)|track.0 as isize==msg.lParam.0) {
                let old=control.get(&value);let next=unsafe{SendMessageW(*track,TBM_GETPOS,None,None)}.0 as i32;
                control.set(&mut value,next);
                if control.get(&value)!=old {
                    let display=wide(&format!("{}{}",control.get(&value),control.unit()));
                    unsafe{let _=SetWindowTextW(*number,PCWSTR(display.as_ptr()));}apply(&value);
                }
            }
            continue;
        }
        if msg.message==WM_KEYDOWN && msg.wParam.0==0x1b && (msg.hwnd==hwnd||unsafe{IsChild(hwnd,msg.hwnd)}.as_bool()){break;}
        if unsafe{IsWindow(Some(hwnd))}.as_bool() && unsafe{IsDialogMessageW(hwnd,&msg)}.as_bool(){continue;}
        unsafe{let _=TranslateMessage(&msg);DispatchMessageW(&msg);}
        if !unsafe{IsWindow(Some(hwnd))}.as_bool(){break;}
    }
}
