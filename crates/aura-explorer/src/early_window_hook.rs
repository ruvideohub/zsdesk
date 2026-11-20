//! Show notifications from a small, inert native hook DLL. The DLL posts, never sends:
//! a component injected into every process must not make other applications wait for us.
use anyhow::{Context,Result};
use windows::{core::{s,PCWSTR},Win32::{Foundation::{FreeLibrary,HINSTANCE,HMODULE,HWND},System::LibraryLoader::{GetModuleHandleW,GetProcAddress,LoadLibraryExW,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,LOAD_LIBRARY_SEARCH_SYSTEM32},UI::{Accessibility::{SetWinEventHook,UnhookWinEvent,HWINEVENTHOOK},WindowsAndMessaging::*}}};
pub(crate) const MESSAGE:u32=WM_APP+119;
pub(crate) struct Hook{window:HWND,class:PCWSTR,instance:HINSTANCE,module:HMODULE,event:HWINEVENTHOOK}
impl Hook{
    pub fn install(class:PCWSTR,proc:WNDPROC,explorer:bool)->Result<Self>{
        let instance=HINSTANCE(unsafe{GetModuleHandleW(None)}?.0);
        anyhow::ensure!(unsafe{RegisterClassW(&WNDCLASSW{hInstance:instance,lpszClassName:class,lpfnWndProc:proc,..Default::default()})}!=0,"early-window endpoint registration");
        let mut result=Self{window:HWND::default(),class,instance,module:HMODULE::default(),event:HWINEVENTHOOK::default()};
        result.window=unsafe{CreateWindowExW(WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE,class,None,WS_POPUP,-30000,-30000,1,1,None,None,Some(instance),None)}?;
        let path=std::env::current_exe()?.parent().context("executable directory")?.join(env!("AURA_WINDOW_HOOK"));
        let path:Vec<u16>=path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        result.module=unsafe{LoadLibraryExW(PCWSTR(path.as_ptr()),None,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32)}?;
        let event=unsafe{GetProcAddress(result.module,if explorer{s!("AuraExplorerExplorerEvent")}else{s!("AuraExplorerDialogEvent")})}.context("early-window event export")?;
        result.event=unsafe{SetWinEventHook(EVENT_OBJECT_SHOW,EVENT_OBJECT_SHOW,Some(result.module),Some(std::mem::transmute(event)),0,0,WINEVENT_INCONTEXT|WINEVENT_SKIPOWNPROCESS)};
        anyhow::ensure!(!result.event.0.is_null(),"window show event hook");
        // No WH_CBT hook here. It was installed for the whole system, so every
        // activation in every process entered our DLL, and each dialog activation
        // ended in a cross-process wait. The show event above already tells the
        // guard about the windows it takes over.
        Ok(result)
    }
}
impl Drop for Hook{fn drop(&mut self){unsafe{
    if !self.event.0.is_null(){let _=UnhookWinEvent(self.event);}
    if !self.window.0.is_null(){let _=DestroyWindow(self.window);}
    let _=UnregisterClassW(self.class,Some(self.instance));
    if !self.module.0.is_null(){let _=FreeLibrary(self.module);}
}}}
