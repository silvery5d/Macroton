// Diagnostic only: enumerate Windows OpenGL pixel formats in the exact Wine prefix.
#include <stdio.h>
#include <windows.h>

int main(void) {
    WNDCLASSA window_class = {0};
    HWND window;
    window_class.style = CS_OWNDC;
    window_class.lpfnWndProc = DefWindowProcA;
    window_class.hInstance = GetModuleHandle(NULL);
    window_class.lpszClassName = "MacrotonWGLProbe";
    if (!RegisterClassA(&window_class)) {
        fprintf(stderr, "RegisterClass failed: %lu\n", GetLastError());
        return 2;
    }
    window = CreateWindowExA(0, window_class.lpszClassName, "Macroton WGL probe",
                             WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT,
                             320, 240, NULL, NULL, window_class.hInstance, NULL);
    if (window) {
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);
    }
    HDC dc = window ? GetDC(window) : NULL;
    HMODULE opengl = LoadLibraryA("opengl32.dll");
    PIXELFORMATDESCRIPTOR pfd = {0};
    int count;
    int usable = 0;
    int index;
    if (!dc) {
        fprintf(stderr, "CreateWindow/GetDC failed: %lu\n", GetLastError());
        return 2;
    }
    pfd.nSize = sizeof(pfd);
    pfd.nVersion = 1;
    pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
    pfd.iPixelType = PFD_TYPE_RGBA;
    pfd.cColorBits = 24;
    printf("opengl32_loaded=%d\n", opengl != NULL);
    printf("chosen_pixel_format=%d\n", ChoosePixelFormat(dc, &pfd));
    count = DescribePixelFormat(dc, 1, sizeof(pfd), &pfd);
    printf("pixel_format_count=%d\n", count);
    for (index = 1; index <= count; index++) {
        if (!DescribePixelFormat(dc, index, sizeof(pfd), &pfd)) continue;
        if ((pfd.dwFlags & PFD_DRAW_TO_WINDOW) &&
            (pfd.dwFlags & PFD_SUPPORT_OPENGL) &&
            (pfd.dwFlags & PFD_DOUBLEBUFFER) &&
            pfd.iPixelType == PFD_TYPE_RGBA) {
            usable++;
            if (usable <= 4) {
                printf("usable_index=%d flags=0x%08lx color=%u depth=%u stencil=%u\n",
                       index, (unsigned long)pfd.dwFlags, pfd.cColorBits,
                       pfd.cDepthBits, pfd.cStencilBits);
            }
        }
    }
    printf("usable_count=%d\n", usable);
    ReleaseDC(window, dc);
    DestroyWindow(window);
    UnregisterClassA(window_class.lpszClassName, window_class.hInstance);
    if (opengl) FreeLibrary(opengl);
    return usable ? 0 : 1;
}
