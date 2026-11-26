//! Fences appearance settings using standard Windows controls. Native trackbars
//! expose their names, values and ranges to accessibility tools and honor high
//! contrast and keyboard settings without a dependency on Dock.
use std::cell::Cell;
use aura_fences_model::{appearance::Control, desktop::DesktopSettings};
use windows::{core::{w,PCWSTR}, Win32::{
    Foundation::{HWND,LPARAM,LRESULT,POINT,WPARAM},
    Graphics::Gdi::{GetMonitorInfoW,MonitorFromPoint,GetStockObject,DEFAULT_GUI_FONT,COLOR_WINDOW,MONITORINFO,MONITOR_DEFAULTTONEAREST},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::{InitCommonControlsEx,INITCOMMONCONTROLSEX,ICC_BAR_CLASSES,TRACKBAR_CLASSW,TBS_HORZ,TBS_NOTICKS,TBM_SETPOS,TBM_SETRANGEMIN,TBM_SETRANGEMAX},
        HiDpi::{GetDpiForMonitor,MDT_EFFECTIVE_DPI}, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
}};
const CLASS:PCWSTR=w!("AuraFences.AppearanceSettings");
const TBM_GETPOS:u32=WM_USER;
const CHANGE:u32=WM_APP+705;
const FINISH:u32=WM_APP+706;
const BLUR_ID:usize=200;
const DONE_ID:usize=201;
thread_local!{static OPEN:Cell<bool>=const{Cell::new(false)};}

unsafe extern "system" fn proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT {
    match msg {
        WM_HSCROLL => {let _=unsafe{PostMessageW(Some(hwnd),CHANGE,WPARAM(0),lp)};LRESULT(0)}
        WM_COMMAND if wp.0 & 0xffff == BLUR_ID => {let _=unsafe{PostMessageW(Some(hwnd),CHANGE,WPARAM(BLUR_ID),LPARAM(0))};LRESULT(0)}
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
pub(super) fn show(owner:HWND,point:POINT,mut value:DesktopSettings,mut apply:impl FnMut(&DesktopSettings)) {
    if OPEN.with(|v|v.replace(true)){return;}let _open=OpenGuard;
    if !register(){return;}
    let monitor=unsafe{MonitorFromPoint(point,MONITOR_DEFAULTTONEAREST)};
    let mut info=MONITORINFO{cbSize:std::mem::size_of::<MONITORINFO>() as u32,..Default::default()};
    if !unsafe{GetMonitorInfoW(monitor,&mut info)}.as_bool(){return;}
    let(mut dx,mut dy)=(96,96);
    let dpi=if unsafe{GetDpiForMonitor(monitor,MDT_EFFECTIVE_DPI,&mut dx,&mut dy)}.is_ok(){dx.max(96)}else{96};
    let work=info.rcWork;let scale=(dpi as f32/96.0).min((work.right-work.left) as f32/520.0).min((work.bottom-work.top) as f32/510.0).max(0.5);
    let px=|x:i32|(x as f32*scale).round() as i32;
    let (width,height)=(px(500),px(490));
    let left=(point.x-width/2).clamp(work.left,(work.right-width).max(work.left));
    let top=(point.y-height/2).clamp(work.top,(work.bottom-height).max(work.top));
    let Ok(instance)=(unsafe{GetModuleHandleW(None)}) else{return};
    let title=text("格子外观设置");
    let Ok(hwnd)=(unsafe{CreateWindowExW(WS_EX_CONTROLPARENT,CLASS,PCWSTR(title.as_ptr()),WS_POPUP|WS_CAPTION|WS_SYSMENU|WS_CLIPCHILDREN,left,top,width,height,Some(owner),None,Some(instance.into()),None)})else{return};
    let _window=Window(hwnd);
    let child=|class:PCWSTR,title:&[u16],style:WINDOW_STYLE,id:usize,x:i32,y:i32,w:i32,h:i32|->Option<HWND>{
        let result=unsafe{CreateWindowExW(WINDOW_EX_STYLE(0),class,PCWSTR(title.as_ptr()),WS_CHILD|WS_VISIBLE|style,px(x),px(y),px(w),px(h),Some(hwnd),Some(HMENU(id as *mut _)),Some(instance.into()),None)}.ok()?;
        let font=unsafe{GetStockObject(DEFAULT_GUI_FONT)};
        unsafe{SendMessageW(result,WM_SETFONT,Some(WPARAM(font.0 as usize)),Some(LPARAM(1)));}
        Some(result)
    };
    let caption=text("更改立即生效");
    if child(w!("STATIC"),&caption,WINDOW_STYLE(0),0,20,12,450,22).is_none(){return;}
    let mut sliders=Vec::new();
    for (i,control) in Control::ALL[..6].iter().copied().enumerate(){
        let y=42+i as i32*53;let label=text(control.label());
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
    let Some(checkbox)=child(w!("BUTTON"),&blur,WS_TABSTOP|WINDOW_STYLE(BS_AUTOCHECKBOX as u32),BLUR_ID,20,365,320,28) else{return};
    unsafe{SendMessageW(checkbox,BM_SETCHECK,Some(WPARAM(value.blur as usize)),None);}
    let done=text("完成");
    if child(w!("BUTTON"),&done,WS_TABSTOP|WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),DONE_ID,355,403,110,30).is_none(){return;}
    unsafe{let _=ShowWindow(hwnd,SW_SHOWNORMAL);let _=SetForegroundWindow(hwnd);let _=SetFocus(Some(sliders[0].1));}
    loop {
        let mut msg=MSG::default();let result=unsafe{GetMessageW(&mut msg,None,0,0)}.0;
        if result==0{unsafe{PostQuitMessage(msg.wParam.0 as i32)};break;}if result<0{break;}
        if msg.hwnd==hwnd && msg.message==FINISH{break;}
        if msg.hwnd==hwnd && msg.message==CHANGE {
            if msg.wParam.0==BLUR_ID {
                let checked=unsafe{SendMessageW(checkbox,BM_GETCHECK,None,None)}.0!=0;
                if checked!=value.blur {Control::Blur.set(&mut value,i32::from(checked));apply(&value);}
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
