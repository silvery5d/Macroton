// Read-only host check matching Wine's WinMac OpenGL renderer enumeration.
#include <ApplicationServices/ApplicationServices.h>
#include <OpenGL/OpenGL.h>
#include <stdio.h>

int main(void) {
    CGLRendererInfoObj info = NULL;
    GLint count = 0;
    CGDirectDisplayID display = CGMainDisplayID();
    CGOpenGLDisplayMask mask = CGDisplayIDToOpenGLDisplayMask(display);
    CGLError error = CGLQueryRendererInfo(mask, &info, &count);
    printf("main_display=%u mask=0x%x cgl_error=%d renderer_count=%d\n",
           display, mask, error, count);
    if (error == kCGLNoError && info) {
        for (GLint index = 0; index < count; index++) {
            const CGLRendererProperty properties[] = {
                kCGLRPRendererID, kCGLRPAccelerated, kCGLRPWindow,
                kCGLRPOnline, kCGLRPBufferModes, kCGLRPColorModes,
                kCGLRPDepthModes, kCGLRPStencilModes
            };
            const char *names[] = {
                "renderer_id", "accelerated", "window", "online",
                "buffer_modes", "color_modes", "depth_modes", "stencil_modes"
            };
            printf("renderer_index=%d", index);
            for (unsigned property = 0; property < sizeof(properties) / sizeof(properties[0]); property++) {
                GLint value = 0;
                CGLError result = CGLDescribeRenderer(info, index, properties[property], &value);
                printf(" %s=%d(error=%d)", names[property], value, result);
            }
            putchar('\n');
        }
        GLint renderer_id = 0;
        CGLDescribeRenderer(info, 0, kCGLRPRendererID, &renderer_id);
        const CGLPixelFormatAttribute candidates[][24] = {
            {kCGLPFAAccelerated, kCGLPFAWindow, 0},
            {kCGLPFAMinimumPolicy, kCGLPFAClosestPolicy, kCGLPFARendererID,
             (CGLPixelFormatAttribute)renderer_id, kCGLPFASingleRenderer,
             kCGLPFAAccelerated, kCGLPFANoRecovery, kCGLPFAWindow, 0},
            {kCGLPFAMinimumPolicy, kCGLPFAClosestPolicy, kCGLPFARendererID,
             (CGLPixelFormatAttribute)renderer_id, kCGLPFASingleRenderer,
             kCGLPFAAccelerated, kCGLPFANoRecovery, kCGLPFADoubleBuffer,
             kCGLPFAAuxBuffers, 0, kCGLPFAColorSize, 24,
             kCGLPFAAlphaSize, 8, kCGLPFADepthSize, 24,
             kCGLPFAStencilSize, 8, kCGLPFAWindow, 0}
        };
        for (unsigned candidate = 0; candidate < sizeof(candidates) / sizeof(candidates[0]); candidate++) {
            CGLPixelFormatObj format = NULL;
            GLint screens = 0;
            CGLError choose_error = CGLChoosePixelFormat((CGLPixelFormatAttribute *)candidates[candidate], &format, &screens);
            printf("candidate=%u choose_error=%d format=%d screens=%d\n",
                   candidate, choose_error, format != NULL, screens);
            if (format) CGLReleasePixelFormat(format);
        }
    }
    if (info) CGLDestroyRendererInfo(info);
    return error == kCGLNoError && count > 0 ? 0 : 1;
}
