/*
 * Macroton Steam CEF wrapper.
 *
 * Steam's CEF helper is multi-process by default. On Wine/macOS Apple Silicon
 * that path can leave the native Steam window black because the renderer/GPU
 * surfaces cross the Wine macOS driver boundary. Keep Valve's helper intact
 * and prepend the two flags known to keep the UI in one CPU-raster process.
 * SPDX-License-Identifier: MIT
 */
#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#include <windows.h>
#include <stdlib.h>
#include <wchar.h>

#define REAL_HELPER L"steamwebhelper_real.exe"
#define EXTRA_FLAGS L"--disable-gpu --single-process"

static const wchar_t *argument_tail(void) {
    const wchar_t *command = GetCommandLineW();
    int quoted = 0;
    if (!command) return L"";
    while (*command) {
        if (*command == L'\"') quoted = !quoted;
        else if (*command == L' ' && !quoted) break;
        ++command;
    }
    while (*command == L' ') ++command;
    return command;
}

int wmain(void) {
    wchar_t self[MAX_PATH];
    DWORD length = GetModuleFileNameW(NULL, self, MAX_PATH);
    if (length == 0 || length >= MAX_PATH) return 1;
    wchar_t *slash = wcsrchr(self, L'\\');
    if (!slash) return 1;
    *(slash + 1) = L'\0';

    size_t real_len = wcslen(self) + wcslen(REAL_HELPER) + 1;
    wchar_t *real = calloc(real_len, sizeof(*real));
    if (!real) return 1;
    wcscpy(real, self);
    wcscat(real, REAL_HELPER);

    const wchar_t *tail = argument_tail();
    size_t command_len = wcslen(real) + wcslen(EXTRA_FLAGS) + wcslen(tail) + 8;
    wchar_t *command = calloc(command_len, sizeof(*command));
    if (!command) {
        free(real);
        return 1;
    }
    _snwprintf(command, command_len, L"\"%ls\" %ls %ls", real, EXTRA_FLAGS, tail);

    STARTUPINFOW startup;
    PROCESS_INFORMATION process;
    ZeroMemory(&startup, sizeof(startup));
    startup.cb = sizeof(startup);
    ZeroMemory(&process, sizeof(process));
    BOOL created = CreateProcessW(
        real, command, NULL, NULL, TRUE, 0, NULL, self, &startup, &process);
    if (!created) {
        free(command);
        free(real);
        return 1;
    }
    WaitForSingleObject(process.hProcess, INFINITE);
    DWORD exit_code = 1;
    GetExitCodeProcess(process.hProcess, &exit_code);
    CloseHandle(process.hProcess);
    CloseHandle(process.hThread);
    free(command);
    free(real);
    return (int)exit_code;
}
