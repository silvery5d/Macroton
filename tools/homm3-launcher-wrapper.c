#include <windows.h>
#include <stdio.h>

int WINAPI WinMain(HINSTANCE instance, HINSTANCE previous, LPSTR command_line, int show) {
    (void)instance;
    (void)previous;
    (void)command_line;
    (void)show;

    char module[MAX_PATH];
    DWORD length = GetModuleFileNameA(NULL, module, MAX_PATH);
    if (length == 0 || length >= MAX_PATH) {
        return 2;
    }
    char *separator = strrchr(module, '\\');
    if (!separator) {
        return 3;
    }
    *separator = '\0';

    char executable[MAX_PATH];
    if (snprintf(executable, MAX_PATH, "%s\\HOMM3 2.0.exe", module) < 0) {
        return 4;
    }
    char arguments[MAX_PATH * 2];
    if (snprintf(arguments, sizeof(arguments),
                 "\"%s\" --fromLauncher --version v1.18 --UBIreport --DOTEMUreport", executable) < 0) {
        return 5;
    }

    STARTUPINFOA startup = {0};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION process = {0};
    if (!CreateProcessA(executable, arguments, NULL, NULL, FALSE, 0, NULL, module,
                        &startup, &process)) {
        return (int)GetLastError();
    }
    WaitForSingleObject(process.hProcess, INFINITE);
    DWORD exit_code = 1;
    GetExitCodeProcess(process.hProcess, &exit_code);
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    return (int)exit_code;
}
