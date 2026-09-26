#define WIN32_LEAN_AND_MEAN
#include <windows.h>

/*
 * Minimal Steam-tracked launcher bridge for games whose legacy launcher is
 * itself incompatible with Wine. Build as a GUI PE32 executable and place it
 * at the launcher's path only in a staged game view. The child inherits the
 * Steam launch context; no Steam API or DRM behavior is emulated here.
 */
int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR command_line,
                    int show_command) {
    (void)instance;
    (void)previous;
    (void)command_line;
    (void)show_command;

    STARTUPINFOW startup = {0};
    PROCESS_INFORMATION process = {0};
    wchar_t child_command[] = L"\"AoK HD.exe\" SKIPINTRO";
    startup.cb = sizeof(startup);

    if (!CreateProcessW(L"AoK HD.exe", child_command, NULL, NULL, FALSE, 0,
                        NULL, NULL, &startup, &process)) {
        wchar_t message[160];
        wsprintfW(message, L"Macroton could not start AoK HD.exe (error %lu).",
                  GetLastError());
        MessageBoxW(NULL, message, L"Macroton launch bridge", MB_OK | MB_ICONERROR);
        return 1;
    }

    /*
     * Steam discovers and tracks the child separately. Do not wait here:
     * Wine's wait path can remain busy after a force-terminated child, leaving
     * a launcher shim at one full CPU even though the game is gone.
     */
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    return 0;
}
