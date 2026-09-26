#define COBJMACROS
#include <d3d10.h>
#include <dxgi.h>
#include <stdio.h>

static void print_adapter(ID3D10Device *device) {
    IDXGIDevice *dxgi_device = NULL;
    IDXGIAdapter *adapter = NULL;
    DXGI_ADAPTER_DESC description;

    HRESULT result = ID3D10Device_QueryInterface(
        device, &IID_IDXGIDevice, (void **)&dxgi_device);
    if (FAILED(result)) {
        printf("adapter_query=failed hresult=0x%08lx\n", (unsigned long)result);
        return;
    }

    result = IDXGIDevice_GetAdapter(dxgi_device, &adapter);
    if (SUCCEEDED(result)) {
        result = IDXGIAdapter_GetDesc(adapter, &description);
    }
    if (SUCCEEDED(result)) {
        printf("adapter_vendor=0x%04x device=0x%04x dedicated_video_memory=%llu\n",
               description.VendorId,
               description.DeviceId,
               (unsigned long long)description.DedicatedVideoMemory);
    } else {
        printf("adapter_query=failed hresult=0x%08lx\n", (unsigned long)result);
    }

    if (adapter != NULL) {
        IDXGIAdapter_Release(adapter);
    }
    IDXGIDevice_Release(dxgi_device);
}

int main(void) {
    ID3D10Device *device = NULL;
    HRESULT result = D3D10CreateDevice(NULL,
                                       D3D10_DRIVER_TYPE_HARDWARE,
                                       NULL,
                                       0,
                                       D3D10_SDK_VERSION,
                                       &device);
    printf("d3d10_create_device=0x%08lx\n", (unsigned long)result);
    if (FAILED(result)) {
        return 1;
    }

    print_adapter(device);
    ID3D10Device_Release(device);
    return 0;
}
