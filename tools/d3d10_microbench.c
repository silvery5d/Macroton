#define COBJMACROS
#include <d3d10.h>
#include <windows.h>
#include <stdio.h>

enum { WIDTH = 1280, HEIGHT = 720, FRAME_COUNT = 180 };

static double elapsed_seconds(LARGE_INTEGER start, LARGE_INTEGER end) {
    LARGE_INTEGER frequency;
    QueryPerformanceFrequency(&frequency);
    return (double)(end.QuadPart - start.QuadPart) / (double)frequency.QuadPart;
}

int main(void) {
    ID3D10Device *device = NULL;
    ID3D10Texture2D *target = NULL;
    ID3D10Texture2D *staging = NULL;
    ID3D10RenderTargetView *view = NULL;
    D3D10_TEXTURE2D_DESC texture_desc = {0};
    D3D10_MAPPED_TEXTURE2D mapped = {0};
    LARGE_INTEGER started;
    LARGE_INTEGER finished;
    unsigned int checksum = 0;
    HRESULT result = D3D10CreateDevice(NULL,
                                       D3D10_DRIVER_TYPE_HARDWARE,
                                       NULL,
                                       0,
                                       D3D10_SDK_VERSION,
                                       &device);
    if (FAILED(result)) {
        printf("benchmark_create_device=0x%08lx\n", (unsigned long)result);
        return 1;
    }

    texture_desc.Width = WIDTH;
    texture_desc.Height = HEIGHT;
    texture_desc.MipLevels = 1;
    texture_desc.ArraySize = 1;
    texture_desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    texture_desc.SampleDesc.Count = 1;
    texture_desc.Usage = D3D10_USAGE_DEFAULT;
    texture_desc.BindFlags = D3D10_BIND_RENDER_TARGET;
    result = ID3D10Device_CreateTexture2D(device, &texture_desc, NULL, &target);
    if (FAILED(result)) {
        printf("benchmark_create_target=0x%08lx\n", (unsigned long)result);
        goto failed;
    }
    result = ID3D10Device_CreateRenderTargetView(device, (ID3D10Resource *)target, NULL, &view);
    if (FAILED(result)) {
        printf("benchmark_create_view=0x%08lx\n", (unsigned long)result);
        goto failed;
    }

    texture_desc.Usage = D3D10_USAGE_STAGING;
    texture_desc.BindFlags = 0;
    texture_desc.CPUAccessFlags = D3D10_CPU_ACCESS_READ;
    result = ID3D10Device_CreateTexture2D(device, &texture_desc, NULL, &staging);
    if (FAILED(result)) {
        printf("benchmark_create_staging=0x%08lx\n", (unsigned long)result);
        goto failed;
    }

    QueryPerformanceCounter(&started);
    for (unsigned int frame = 0; frame < FRAME_COUNT; ++frame) {
        const float color[4] = {
            (float)(frame & 0xffu) / 255.0f,
            (float)((frame * 3u) & 0xffu) / 255.0f,
            (float)((frame * 7u) & 0xffu) / 255.0f,
            1.0f,
        };
        ID3D10Device_ClearRenderTargetView(device, view, color);
        ID3D10Device_CopyResource(device, (ID3D10Resource *)staging, (ID3D10Resource *)target);
        result = ID3D10Texture2D_Map(staging, 0, D3D10_MAP_READ, 0, &mapped);
        if (FAILED(result)) {
            printf("benchmark_map_frame=%u hresult=0x%08lx\n",
                   frame,
                   (unsigned long)result);
            goto failed;
        }
        checksum += ((const unsigned char *)mapped.pData)[0];
        ID3D10Texture2D_Unmap(staging, 0);
    }
    QueryPerformanceCounter(&finished);

    {
        const double seconds = elapsed_seconds(started, finished);
        printf("benchmark_frames=%u width=%u height=%u seconds=%.6f fps=%.3f checksum=%u\n",
               FRAME_COUNT,
               WIDTH,
               HEIGHT,
               seconds,
               (double)FRAME_COUNT / seconds,
               checksum);
    }

    ID3D10Texture2D_Release(staging);
    ID3D10RenderTargetView_Release(view);
    ID3D10Texture2D_Release(target);
    ID3D10Device_Release(device);
    return 0;

failed:
    if (staging != NULL) {
        ID3D10Texture2D_Release(staging);
    }
    if (view != NULL) {
        ID3D10RenderTargetView_Release(view);
    }
    if (target != NULL) {
        ID3D10Texture2D_Release(target);
    }
    ID3D10Device_Release(device);
    return 1;
}
