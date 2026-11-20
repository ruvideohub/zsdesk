#define WIN32_LEAN_AND_MEAN
#include <windows.h>

// No Shell, COM, allocations or background threads run in the caller. The
// recovery process owns every visibility change.
// Nothing here may block the process it is injected into. This used to send the
// notification with SendMessageTimeoutW and wait up to 80 ms for the guard to
// acknowledge; every window show and every dialog activation in every process
// then waited on us, which is exactly what makes Windows flip the pointer into
// the "working in background" cursor. The guard hides the window from its own
// thread a millisecond later, so the notification is posted, not sent.
static HWND receiver_for(bool explorer) {
    // FindWindowW enumerates every top-level window; cache it per process
    // instead of searching again on every single window show.
    static HWND cached_explorer = nullptr;
    static HWND cached_dialog = nullptr;
    HWND* slot = explorer ? &cached_explorer : &cached_dialog;
    if (*slot && IsWindow(*slot)) return *slot;
    *slot = FindWindowW(explorer ? L"AuraExplorer.EarlyExplorer.Guard" : L"AuraExplorer.EarlyDialog.Guard", nullptr);
    return *slot;
}
static void signal(HWND window, bool explorer, UINT event) {
    if (!window || GetAncestor(window, GA_ROOT) != window) return;
    wchar_t name[64];
    if (!GetClassNameW(window, name, 64)) return;
    if (explorer ? (lstrcmpW(name, L"CabinetWClass") && lstrcmpW(name, L"ExploreWClass"))
                 : lstrcmpW(name, L"#32770")) return;
    HWND receiver = receiver_for(explorer);
    if (!receiver) return;
    DWORD receiver_pid = 0;
    GetWindowThreadProcessId(receiver, &receiver_pid);
    if (receiver_pid == GetCurrentProcessId()) return;
    PostMessageW(receiver, WM_APP + 119, reinterpret_cast<WPARAM>(window), event);
}
extern "C" __declspec(dllexport) void CALLBACK AuraExplorerDialogEvent(HWINEVENTHOOK, DWORD event, HWND window, LONG object, LONG child, DWORD, DWORD) {
    if (object == OBJID_WINDOW && child == 0) {
        signal(window, false, event);
        wchar_t probe[1];
        if (event == EVENT_OBJECT_SHOW && GetAncestor(window, GA_ROOT) == window &&
            GetEnvironmentVariableW(L"AURA_DIALOG_FIXTURE", probe, 1) && !GetPropW(window, L"AuraExplorer.Test.FirstShowAlpha")) {
            BYTE alpha = 255; GetLayeredWindowAttributes(window, nullptr, &alpha, nullptr);
            SetPropW(window, L"AuraExplorer.Test.FirstShowAlpha", reinterpret_cast<HANDLE>(static_cast<UINT_PTR>(alpha) + 1));
        }
    }
}
extern "C" __declspec(dllexport) void CALLBACK AuraExplorerExplorerEvent(HWINEVENTHOOK, DWORD event, HWND window, LONG object, LONG child, DWORD, DWORD) {
    if (object == OBJID_WINDOW && child == 0) signal(window, true, event);
}
extern "C" __declspec(dllexport) LRESULT CALLBACK AuraExplorerDialogCbt(int code, WPARAM wp, LPARAM lp) {
    if (code == HCBT_ACTIVATE) signal(reinterpret_cast<HWND>(wp), false, EVENT_OBJECT_SHOW);
    return CallNextHookEx(nullptr, code, wp, lp);
}
extern "C" __declspec(dllexport) LRESULT CALLBACK AuraExplorerExplorerCbt(int code, WPARAM wp, LPARAM lp) {
    if (code == HCBT_ACTIVATE) signal(reinterpret_cast<HWND>(wp), true, EVENT_OBJECT_SHOW);
    return CallNextHookEx(nullptr, code, wp, lp);
}
extern "C" BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }
