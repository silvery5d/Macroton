#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *activate_title = NULL;
static const char *resize_title = NULL;
static const char *send_key_title = NULL;
static const char *move_mouse_title = NULL;
static int click_mouse = 0;
static unsigned int send_virtual_key = 0;
static long mouse_client_x = 0;
static long mouse_client_y = 0;
static int activated = 0;
static int include_hidden = 0;

static BOOL CALLBACK print_child_window(HWND window, LPARAM root_value) {
    char title[512];
    char class_name[256];
    DWORD process_id = 0;
    RECT rectangle = {0};
    HWND parent = GetParent(window);

    GetWindowThreadProcessId(window, &process_id);
    GetWindowTextA(window, title, sizeof(title));
    GetClassNameA(window, class_name, sizeof(class_name));
    GetWindowRect(window, &rectangle);
    printf("child root=%p parent=%p pid=%lu hwnd=%p visible=%d rect=%ld,%ld,%ld,%ld class=%s title=%s\n",
           (void *)root_value, (void *)parent, (unsigned long)process_id,
           (void *)window, IsWindowVisible(window),
           (long)rectangle.left, (long)rectangle.top,
           (long)rectangle.right, (long)rectangle.bottom,
           class_name, title);
    return TRUE;
}

static BOOL CALLBACK print_window(HWND window, LPARAM unused) {
    char title[512];
    char class_name[256];
    DWORD process_id = 0;
    (void)unused;

    if (!include_hidden && !IsWindowVisible(window)) return TRUE;
    GetWindowThreadProcessId(window, &process_id);
    GetWindowTextA(window, title, sizeof(title));
    GetClassNameA(window, class_name, sizeof(class_name));
    {
        RECT rectangle = {0};
        GetWindowRect(window, &rectangle);
        printf("pid=%lu hwnd=%p visible=%d enabled=%d rect=%ld,%ld,%ld,%ld class=%s title=%s\n",
               (unsigned long)process_id, (void *)window, IsWindowVisible(window),
               IsWindowEnabled(window), (long)rectangle.left, (long)rectangle.top,
               (long)rectangle.right, (long)rectangle.bottom, class_name, title);
    }
    EnumChildWindows(window, print_child_window, (LPARAM)window);
    if (activate_title != NULL && strcmp(title, activate_title) == 0) {
        ShowWindow(window, SW_RESTORE);
        BringWindowToTop(window);
        SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        SetWindowPos(window, HWND_NOTOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        SetForegroundWindow(window);
        activated = 1;
    }
    if (resize_title != NULL && strcmp(title, resize_title) == 0) {
        RECT rectangle;
        if (!GetWindowRect(window, &rectangle) ||
            !SetWindowPos(window, NULL, 0, 0,
                          rectangle.right - rectangle.left + 20,
                          rectangle.bottom - rectangle.top + 20,
                          SWP_NOMOVE | SWP_NOZORDER | SWP_SHOWWINDOW)) {
            fprintf(stderr, "could not resize requested window: %s\n", title);
            return FALSE;
        }
        activated = 1;
    }
    if (send_key_title != NULL && strcmp(title, send_key_title) == 0) {
        INPUT inputs[2] = {0};
        ShowWindow(window, SW_RESTORE);
        BringWindowToTop(window);
        SetForegroundWindow(window);
        Sleep(100);
        inputs[0].type = INPUT_KEYBOARD;
        inputs[0].ki.wVk = (WORD)send_virtual_key;
        inputs[1] = inputs[0];
        inputs[1].ki.dwFlags = KEYEVENTF_KEYUP;
        if (SendInput(2, inputs, sizeof(INPUT)) != 2) {
            fprintf(stderr, "could not send requested key to window: %s\n", title);
            return FALSE;
        }
        /* Some games consume their Win32 queue without accepting a synthetic
         * foreground transition from Wine. Mirror the physical input into the
         * selected top-level window, as the mouse probe already does. */
        PostMessageA(window, WM_KEYDOWN, (WPARAM)send_virtual_key, 0);
        Sleep(50);
        PostMessageA(window, WM_KEYUP, (WPARAM)send_virtual_key,
                     (LPARAM)(1UL << 30 | 1UL << 31));
        activated = 1;
    }
    if (move_mouse_title != NULL && strcmp(title, move_mouse_title) == 0) {
        POINT point = {mouse_client_x, mouse_client_y};
        ShowWindow(window, SW_RESTORE);
        BringWindowToTop(window);
        SetForegroundWindow(window);
        if (!ClientToScreen(window, &point) || !SetCursorPos(point.x, point.y)) {
            fprintf(stderr, "could not move mouse into requested window: %s\n", title);
            return FALSE;
        }
        if (click_mouse) {
            INPUT input = {0};
            LPARAM client_point = MAKELPARAM((WORD)mouse_client_x,
                                             (WORD)mouse_client_y);
            input.type = INPUT_MOUSE;
            input.mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
            if (SendInput(1, &input, sizeof(INPUT)) != 1) {
                fprintf(stderr, "could not click requested window: %s\n", title);
                return FALSE;
            }
            Sleep(150);
            input.mi.dwFlags = MOUSEEVENTF_LEFTUP;
            if (SendInput(1, &input, sizeof(INPUT)) != 1) {
                fprintf(stderr, "could not release click in requested window: %s\n", title);
                return FALSE;
            }
            PostMessageA(window, WM_LBUTTONDOWN, MK_LBUTTON, client_point);
            Sleep(50);
            PostMessageA(window, WM_LBUTTONUP, 0, client_point);
        }
        activated = 1;
    }
    return TRUE;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--all") == 0) {
        include_hidden = 1;
    } else if (argc == 3 && strcmp(argv[1], "--activate-title") == 0) {
        activate_title = argv[2];
        include_hidden = 1;
    } else if (argc == 3 && strcmp(argv[1], "--resize-title") == 0) {
        resize_title = argv[2];
        include_hidden = 1;
    } else if (argc == 4 && strcmp(argv[1], "--send-key-title") == 0) {
        char *end = NULL;
        send_key_title = argv[2];
        include_hidden = 1;
        send_virtual_key = (unsigned int)strtoul(argv[3], &end, 0);
        if (!end || *end || send_virtual_key > 0xff) {
            fprintf(stderr, "invalid virtual-key code: %s\n", argv[3]);
            return 2;
        }
    } else if (argc == 5 &&
               (strcmp(argv[1], "--move-mouse-title") == 0 ||
                strcmp(argv[1], "--click-title") == 0)) {
        char *x_end = NULL;
        char *y_end = NULL;
        move_mouse_title = argv[2];
        include_hidden = 1;
        click_mouse = strcmp(argv[1], "--click-title") == 0;
        mouse_client_x = strtol(argv[3], &x_end, 0);
        mouse_client_y = strtol(argv[4], &y_end, 0);
        if (!x_end || *x_end || !y_end || *y_end ||
            mouse_client_x < 0 || mouse_client_y < 0) {
            fprintf(stderr, "invalid client coordinates: %s,%s\n", argv[3], argv[4]);
            return 2;
        }
    } else if (argc != 1) {
        fprintf(stderr, "usage: window_probe [--all | --activate-title TITLE | --resize-title TITLE | --send-key-title TITLE VK | --move-mouse-title TITLE X Y | --click-title TITLE X Y]\n");
        return 2;
    }
    if (!EnumWindows(print_window, 0)) {
        fprintf(stderr, "EnumWindows failed: %lu\n", (unsigned long)GetLastError());
        return 1;
    }
    if ((activate_title != NULL || resize_title != NULL || send_key_title != NULL ||
         move_mouse_title != NULL) && !activated) {
        fprintf(stderr, "could not find requested window: %s\n",
                activate_title != NULL ? activate_title :
                (resize_title != NULL ? resize_title :
                (send_key_title != NULL ? send_key_title : move_mouse_title)));
        return 3;
    }
    return 0;
}
