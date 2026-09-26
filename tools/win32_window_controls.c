#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <wchar.h>

typedef struct {
    const wchar_t *window_title;
    const wchar_t *control_text;
    HWND match;
    int matches;
} ClickContext;

typedef struct {
    const wchar_t *window_title;
    HWND match;
    int matches;
} WindowContext;

typedef struct {
    const wchar_t *class_name;
    HWND match;
    int matches;
} ChildClassContext;

static void print_window(HWND window, int depth) {
    wchar_t title[512] = L"";
    wchar_t class_name[256] = L"";
    GetWindowTextW(window, title, 511);
    GetClassNameW(window, class_name, 255);
    DWORD process_id = 0;
    GetWindowThreadProcessId(window, &process_id);
    RECT client = {0};
    GetClientRect(window, &client);
    wprintf(L"%*lsHWND=%p PID=%lu visible=%d enabled=%d client=%ldx%ld class=%ls text=%ls\n",
            depth * 2, L"", window, (unsigned long)process_id,
            IsWindowVisible(window), IsWindowEnabled(window),
            client.right - client.left, client.bottom - client.top,
            class_name, title);
}

static BOOL CALLBACK list_child(HWND window, LPARAM parameter) {
    int depth = (int)parameter;
    print_window(window, depth);
    return TRUE;
}

static BOOL CALLBACK list_top(HWND window, LPARAM parameter) {
    (void)parameter;
    if (!IsWindowVisible(window)) return TRUE;
    print_window(window, 0);
    EnumChildWindows(window, list_child, 1);
    return TRUE;
}

static BOOL CALLBACK find_control(HWND window, LPARAM parameter) {
    ClickContext *context = (ClickContext *)parameter;
    wchar_t text[512] = L"";
    GetWindowTextW(window, text, 511);
    if (IsWindowVisible(window) && IsWindowEnabled(window)
        && _wcsicmp(text, context->control_text) == 0) {
        context->match = window;
        context->matches++;
    }
    return TRUE;
}

static BOOL CALLBACK find_top(HWND window, LPARAM parameter) {
    ClickContext *context = (ClickContext *)parameter;
    wchar_t title[512] = L"";
    GetWindowTextW(window, title, 511);
    if (IsWindowVisible(window) && _wcsicmp(title, context->window_title) == 0) {
        EnumChildWindows(window, find_control, parameter);
    }
    return TRUE;
}

static BOOL CALLBACK find_titled_window(HWND window, LPARAM parameter) {
    WindowContext *context = (WindowContext *)parameter;
    wchar_t title[512] = L"";
    GetWindowTextW(window, title, 511);
    if (IsWindowVisible(window) && IsWindowEnabled(window)
        && _wcsicmp(title, context->window_title) == 0) {
        context->match = window;
        context->matches++;
    }
    return TRUE;
}

static BOOL CALLBACK find_child_class(HWND window, LPARAM parameter) {
    ChildClassContext *context = (ChildClassContext *)parameter;
    wchar_t class_name[256] = L"";
    GetClassNameW(window, class_name, 255);
    if (IsWindowVisible(window) && IsWindowEnabled(window)
        && _wcsicmp(class_name, context->class_name) == 0) {
        context->match = window;
        context->matches++;
    }
    return TRUE;
}

static int parse_coordinate(const wchar_t *text, long *value) {
    wchar_t *end = NULL;
    *value = wcstol(text, &end, 10);
    return end != text && *end == L'\0' && *value >= 0 && *value <= 65535;
}

int wmain(int argc, wchar_t **argv) {
    if (argc == 2 && wcscmp(argv[1], L"list") == 0) {
        EnumWindows(list_top, 0);
        return 0;
    }
    if (argc == 4 && wcscmp(argv[1], L"click") == 0) {
        ClickContext context = {argv[2], argv[3], NULL, 0};
        EnumWindows(find_top, (LPARAM)&context);
        if (context.matches != 1) {
            fwprintf(stderr, L"refusing click: expected exactly one match, found %d\n",
                     context.matches);
            return 1;
        }
        SendMessageW(context.match, BM_CLICK, 0, 0);
        wprintf(L"clicked window=%ls control=%ls matches=1\n",
                context.window_title, context.control_text);
        return 0;
    }
    if ((argc == 5 && wcscmp(argv[1], L"click-client") == 0)
        || (argc == 6 && wcscmp(argv[1], L"click-child") == 0)) {
        int child = wcscmp(argv[1], L"click-child") == 0;
        long x, y;
        if (!parse_coordinate(argv[child ? 4 : 3], &x)
            || !parse_coordinate(argv[child ? 5 : 4], &y)) {
            fputws(L"invalid client coordinates\n", stderr);
            return 2;
        }
        WindowContext context = {argv[2], NULL, 0};
        EnumWindows(find_titled_window, (LPARAM)&context);
        if (context.matches != 1) {
            fwprintf(stderr, L"refusing click: expected exactly one titled window, found %d\n",
                     context.matches);
            return 1;
        }
        HWND target = context.match;
        if (child) {
            ChildClassContext child_context = {argv[3], NULL, 0};
            EnumChildWindows(context.match, find_child_class, (LPARAM)&child_context);
            if (child_context.matches != 1) {
                fwprintf(stderr, L"refusing click: expected exactly one child class match, found %d\n",
                         child_context.matches);
                return 1;
            }
            target = child_context.match;
        }
        RECT client = {0};
        if (!GetClientRect(target, &client)
            || x >= client.right || y >= client.bottom) {
            fwprintf(stderr, L"refusing click: point %ld,%ld is outside client %ldx%ld\n",
                     x, y, client.right, client.bottom);
            return 1;
        }
        LPARAM point = MAKELPARAM((WORD)x, (WORD)y);
        SendMessageW(target, WM_MOUSEMOVE, 0, point);
        SendMessageW(target, WM_LBUTTONDOWN, MK_LBUTTON, point);
        SendMessageW(target, WM_LBUTTONUP, 0, point);
        wprintf(L"clicked window=%ls client=%ld,%ld matches=1\n", context.window_title, x, y);
        return 0;
    }
    fputws(L"usage: win32_window_controls.exe list | click WINDOW_TITLE CONTROL_TEXT | click-client WINDOW_TITLE X Y | click-child WINDOW_TITLE CHILD_CLASS X Y\n",
           stderr);
    return 2;
}
