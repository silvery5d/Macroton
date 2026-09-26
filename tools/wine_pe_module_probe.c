/* Read-only Win32 module and fixed-address probe for a short-lived Wine game. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <tlhelp32.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static DWORD find_process(const char *name)
{
    HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    PROCESSENTRY32 entry;
    DWORD pid = 0;
    if (snapshot == INVALID_HANDLE_VALUE) return 0;
    memset(&entry, 0, sizeof(entry));
    entry.dwSize = sizeof(entry);
    if (Process32First(snapshot, &entry)) {
        do {
            if (_stricmp(entry.szExeFile, name) == 0) {
                pid = entry.th32ProcessID;
                break;
            }
        } while (Process32Next(snapshot, &entry));
    }
    CloseHandle(snapshot);
    return pid;
}

static void probe_address_region(DWORD pid, uintptr_t address)
{
    static DWORD last_pid = 0;
    static DWORD last_state = 0;
    static DWORD last_protect = 0;
    static DWORD last_error = 0;
    static int bytes_reported = 0;
    static unsigned char last_bytes[16];
    static unsigned int byte_changes = 0;
    HANDLE process;
    MEMORY_BASIC_INFORMATION region;
    unsigned char bytes[16];
    SIZE_T count = 0;
    unsigned int i;
    if (last_pid != pid) {
        last_pid = pid;
        last_state = 0;
        last_protect = 0;
        last_error = 0;
        bytes_reported = 0;
        byte_changes = 0;
    }
    process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, pid);
    if (!process) {
        DWORD error = GetLastError();
        if (last_error != error) {
            printf("address_open_error pid=%lu error=%lu\n",
                   (unsigned long)pid, (unsigned long)error);
            fflush(stdout);
            last_error = error;
        }
        return;
    }
    if (!VirtualQueryEx(process, (LPCVOID)address, &region, sizeof(region))) {
        DWORD error = GetLastError();
        if (last_error != error) {
            printf("address_query_error pid=%lu error=%lu\n",
                   (unsigned long)pid, (unsigned long)error);
            fflush(stdout);
            last_error = error;
        }
        CloseHandle(process);
        return;
    }
    if (last_state != region.State || last_protect != region.Protect) {
        printf("address_region pid=%lu base=0x%08lx size=0x%08lx state=0x%lx protect=0x%lx type=0x%lx\n",
               (unsigned long)pid, (unsigned long)(uintptr_t)region.BaseAddress,
               (unsigned long)region.RegionSize, (unsigned long)region.State,
               (unsigned long)region.Protect, (unsigned long)region.Type);
        fflush(stdout);
        last_state = region.State;
        last_protect = region.Protect;
    }
    if (region.State == MEM_COMMIT &&
        ReadProcessMemory(process, (LPCVOID)address, bytes, sizeof(bytes), &count) &&
        count == sizeof(bytes)) {
        if ((!bytes_reported || memcmp(last_bytes, bytes, sizeof(bytes)) != 0) &&
            byte_changes < 20) {
            printf("address_bytes pid=%lu change=%u count=%lu hex=",
                   (unsigned long)pid, byte_changes, (unsigned long)count);
            for (i = 0; i < count; ++i) printf("%02x", bytes[i]);
            printf("\n");
            fflush(stdout);
            memcpy(last_bytes, bytes, sizeof(bytes));
            bytes_reported = 1;
            ++byte_changes;
        }
    }
    CloseHandle(process);
}

static int find_address_module(DWORD pid, uintptr_t address)
{
    HANDLE snapshot;
    MODULEENTRY32 entry;
    unsigned int retry;
    static DWORD reported_steamclient_pid = 0;
    static uintptr_t reported_steamclient_base = 0;
    static DWORD reported_snapshot_error_pid = 0;
    static DWORD reported_module_count_pid = 0;
    static unsigned int last_module_count = 0;
    unsigned int module_count = 0;
    for (retry = 0; retry < 5; ++retry) {
        snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if (snapshot != INVALID_HANDLE_VALUE) break;
        if (GetLastError() != ERROR_BAD_LENGTH) {
            if (reported_snapshot_error_pid != pid) {
                printf("module_snapshot_error pid=%lu error=%lu\n",
                       (unsigned long)pid, (unsigned long)GetLastError());
                fflush(stdout);
                reported_snapshot_error_pid = pid;
            }
            return 0;
        }
        Sleep(10);
    }
    if (snapshot == INVALID_HANDLE_VALUE) {
        if (reported_snapshot_error_pid != pid) {
            printf("module_snapshot_error pid=%lu error=%lu\n",
                   (unsigned long)pid, (unsigned long)GetLastError());
            fflush(stdout);
            reported_snapshot_error_pid = pid;
        }
        return 0;
    }
    memset(&entry, 0, sizeof(entry));
    entry.dwSize = sizeof(entry);
    if (Module32First(snapshot, &entry)) {
        do {
            ++module_count;
            uintptr_t base = (uintptr_t)entry.modBaseAddr;
            uintptr_t end = base + (uintptr_t)entry.modBaseSize;
            if (_stricmp(entry.szModule, "steamclient.dll") == 0 &&
                (reported_steamclient_pid != pid || reported_steamclient_base != base)) {
                printf("steamclient_seen pid=%lu base=0x%08lx size=0x%08lx address_inside=%s path=%s\n",
                       (unsigned long)pid, (unsigned long)base,
                       (unsigned long)entry.modBaseSize,
                       address >= base && address < end ? "yes" : "no",
                       entry.szExePath);
                fflush(stdout);
                reported_steamclient_pid = pid;
                reported_steamclient_base = base;
            }
            if (address >= base && address < end) {
                HANDLE process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, pid);
                unsigned char bytes[16];
                SIZE_T count = 0;
                unsigned int i;
                printf("pid=%lu module=%s base=0x%08lx size=0x%08lx rva=0x%08lx path=%s\n",
                       (unsigned long)pid, entry.szModule, (unsigned long)base,
                       (unsigned long)entry.modBaseSize, (unsigned long)(address - base),
                       entry.szExePath);
                if (process && ReadProcessMemory(process, (LPCVOID)address,
                                                 bytes, sizeof(bytes), &count)) {
                    printf("bytes_at_address=");
                    for (i = 0; i < count; ++i) printf("%02x", bytes[i]);
                    printf("\n");
                } else {
                    printf("read_error=%lu\n", (unsigned long)GetLastError());
                }
                if (process) CloseHandle(process);
                CloseHandle(snapshot);
                fflush(stdout);
                return 1;
            }
        } while (Module32Next(snapshot, &entry));
    } else if (reported_snapshot_error_pid != pid) {
        printf("module_first_error pid=%lu error=%lu\n",
               (unsigned long)pid, (unsigned long)GetLastError());
        fflush(stdout);
        reported_snapshot_error_pid = pid;
    }
    if (module_count &&
        (reported_module_count_pid != pid || last_module_count != module_count)) {
        printf("module_snapshot pid=%lu count=%u address_found=no\n",
               (unsigned long)pid, module_count);
        fflush(stdout);
        reported_module_count_pid = pid;
        last_module_count = module_count;
    }
    CloseHandle(snapshot);
    return 0;
}

int main(int argc, char **argv)
{
    char *end;
    unsigned long address;
    unsigned int attempt;
    unsigned long wait_seconds = 60;
    int saw_process = 0;
    unsigned int missing_after_seen = 0;
    if (argc != 3 && argc != 4) {
        fprintf(stderr, "usage: wine_pe_module_probe.exe GAME_EXE_NAME ADDRESS [WAIT_SECONDS]\n");
        return 2;
    }
    address = strtoul(argv[2], &end, 0);
    if (end == argv[2] || *end != '\0' || address == 0) {
        fprintf(stderr, "ADDRESS must be a nonzero 32-bit integer\n");
        return 2;
    }
    if (argc == 4) {
        wait_seconds = strtoul(argv[3], &end, 10);
        if (end == argv[3] || *end != '\0' || wait_seconds == 0 || wait_seconds > 300) {
            fprintf(stderr, "WAIT_SECONDS must be 1..300\n");
            return 2;
        }
    }
    for (attempt = 0; attempt < wait_seconds * 20; ++attempt) {
        DWORD pid = find_process(argv[1]);
        if (pid) {
            if (!saw_process) {
                printf("process_seen pid=%lu name=%s\n", (unsigned long)pid, argv[1]);
                fflush(stdout);
            }
            saw_process = 1;
            missing_after_seen = 0;
            probe_address_region(pid, (uintptr_t)address);
            if (find_address_module(pid, (uintptr_t)address)) return 0;
        } else if (saw_process && ++missing_after_seen >= 10) {
            fprintf(stderr, "module_not_found address=0x%08lx process_observed=yes process_exited=yes\n",
                    address);
            return 1;
        }
        Sleep(50);
    }
    fprintf(stderr, "module_not_found address=0x%08lx process_observed=%s\n",
            address, saw_process ? "yes" : "no");
    return 1;
}
