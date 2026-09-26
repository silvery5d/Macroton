#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d9.h>
#include <stdio.h>

static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam,
                                    LPARAM lparam) {
    return DefWindowProcA(window, message, wparam, lparam);
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    setvbuf(stderr, NULL, _IONBF, 0);
    WNDCLASSA window_class = {0};
    window_class.lpfnWndProc = window_proc;
    window_class.hInstance = GetModuleHandleA(NULL);
    window_class.lpszClassName = "MacrotonD3D9Probe";
    if (!RegisterClassA(&window_class) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        fprintf(stderr, "register_class_error=%lu\n", GetLastError());
        return 2;
    }

    HWND window = CreateWindowA(window_class.lpszClassName, "Macroton D3D9 Probe",
                                WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT,
                                320, 240, NULL, NULL, window_class.hInstance, NULL);
    if (!window) {
        fprintf(stderr, "create_window_error=%lu\n", GetLastError());
        return 3;
    }
    ShowWindow(window, SW_SHOW);
    UpdateWindow(window);

    HDC device_context = GetDC(window);
    if (device_context) {
        PatBlt(device_context, 0, 0, 1, 1, BLACKNESS);
        GdiFlush();
        ReleaseDC(window, device_context);
    }
    (void)GetSystemMetrics(SM_CXSCREEN);

    MSG message;
    while (PeekMessageA(&message, NULL, 0, 0, PM_REMOVE)) {
        TranslateMessage(&message);
        DispatchMessageA(&message);
    }
    /* Give Wine's macOS display driver time to publish its Vulkan WSI hooks.
     * Without this, a tiny process can query vulkan-1 before winemac.drv has
     * finished attaching and intermittently miss VK_KHR_win32_surface. */
    Sleep(5000);
    while (PeekMessageA(&message, NULL, 0, 0, PM_REMOVE)) {
        TranslateMessage(&message);
        DispatchMessageA(&message);
    }

    printf("d3d9_create9_begin\n");
    IDirect3D9 *d3d = Direct3DCreate9(D3D_SDK_VERSION);
    if (!d3d) {
        fprintf(stderr, "direct3d_create9=null\n");
        DestroyWindow(window);
        return 4;
    }

    D3DADAPTER_IDENTIFIER9 identifier = {0};
    HRESULT identifier_result = d3d->lpVtbl->GetAdapterIdentifier(
        d3d, D3DADAPTER_DEFAULT, 0, &identifier);
    printf("adapter_identifier=0x%08lx\n", (unsigned long)identifier_result);
    if (SUCCEEDED(identifier_result)) {
        printf("adapter_description=%s\n", identifier.Description);
        printf("adapter_vendor=0x%04lx\n", (unsigned long)identifier.VendorId);
        printf("adapter_device=0x%04lx\n", (unsigned long)identifier.DeviceId);
    }

    D3DPRESENT_PARAMETERS present = {0};
    present.Windowed = TRUE;
    present.SwapEffect = D3DSWAPEFFECT_DISCARD;
    present.hDeviceWindow = window;
    present.BackBufferWidth = 320;
    present.BackBufferHeight = 240;
    present.BackBufferFormat = D3DFMT_UNKNOWN;
    present.PresentationInterval = D3DPRESENT_INTERVAL_IMMEDIATE;

    IDirect3DDevice9 *device = NULL;
    printf("d3d9_create_device_begin\n");
    HRESULT create_result = d3d->lpVtbl->CreateDevice(
        d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, window,
        D3DCREATE_HARDWARE_VERTEXPROCESSING | D3DCREATE_MULTITHREADED,
        &present, &device);
    printf("d3d9_create_device=0x%08lx\n", (unsigned long)create_result);

    HRESULT clear_result = E_FAIL;
    HRESULT present_result = E_FAIL;
    if (device) {
        printf("d3d9_clear_begin\n");
        clear_result = device->lpVtbl->Clear(device, 0, NULL, D3DCLEAR_TARGET,
                                             D3DCOLOR_XRGB(24, 48, 96), 1.0f, 0);
        printf("d3d9_present_begin\n");
        present_result = device->lpVtbl->Present(device, NULL, NULL, NULL, NULL);
        printf("d3d9_clear=0x%08lx\n", (unsigned long)clear_result);
        printf("d3d9_present=0x%08lx\n", (unsigned long)present_result);
        device->lpVtbl->Release(device);
    }
    d3d->lpVtbl->Release(d3d);
    DestroyWindow(window);
    return SUCCEEDED(create_result) && SUCCEEDED(clear_result) && SUCCEEDED(present_result) ? 0 : 5;
}
