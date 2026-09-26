#include <ApplicationServices/ApplicationServices.h>
#include <ImageIO/ImageIO.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>

typedef CGImageRef (*create_window_image_fn)(CGRect, CGWindowListOption, CGWindowID,
                                              CGWindowImageOption);

static void print_cf_string(CFStringRef value)
{
    char buffer[1024];
    if (value && CFStringGetCString(value, buffer, sizeof(buffer), kCFStringEncodingUTF8))
        fputs(buffer, stdout);
}

static int list_windows(void)
{
    CFArrayRef windows = CGWindowListCopyWindowInfo(kCGWindowListOptionAll,
                                                     kCGNullWindowID);
    CFIndex count;
    if (!windows) return 1;
    count = CFArrayGetCount(windows);
    for (CFIndex index = 0; index < count; ++index)
    {
        CFDictionaryRef entry = CFArrayGetValueAtIndex(windows, index);
        CFNumberRef number = CFDictionaryGetValue(entry, kCGWindowNumber);
        CFNumberRef layer = CFDictionaryGetValue(entry, kCGWindowLayer);
        CFStringRef owner = CFDictionaryGetValue(entry, kCGWindowOwnerName);
        CFStringRef name = CFDictionaryGetValue(entry, kCGWindowName);
        CFDictionaryRef bounds = CFDictionaryGetValue(entry, kCGWindowBounds);
        CGRect rectangle = CGRectZero;
        int window_id = 0, window_layer = 0;
        if (number) CFNumberGetValue(number, kCFNumberIntType, &window_id);
        if (layer) CFNumberGetValue(layer, kCFNumberIntType, &window_layer);
        if (bounds) CGRectMakeWithDictionaryRepresentation(bounds, &rectangle);
        printf("id=%d layer=%d rect=%.0f,%.0f,%.0f,%.0f owner=", window_id,
               window_layer, rectangle.origin.x, rectangle.origin.y,
               rectangle.size.width, rectangle.size.height);
        print_cf_string(owner);
        fputs(" title=", stdout);
        print_cf_string(name);
        fputc('\n', stdout);
    }
    CFRelease(windows);
    return 0;
}

int main(int argc, char **argv)
{
    create_window_image_fn create_image;
    CGImageDestinationRef destination;
    CGImageRef image;
    CFURLRef url;
    CGWindowID window_id;

    if (argc == 2 && strcmp(argv[1], "--list") == 0)
        return list_windows();

    if (argc != 3)
    {
        fprintf(stderr, "usage: window_capture --list | WINDOW_ID OUTPUT.png\n");
        return 2;
    }

    window_id = (CGWindowID)strtoul(argv[1], NULL, 10);
    create_image = (create_window_image_fn)dlsym(RTLD_DEFAULT, "CGWindowListCreateImage");
    if (!create_image)
    {
        fprintf(stderr, "CGWindowListCreateImage is unavailable\n");
        return 1;
    }

    image = create_image(CGRectNull, kCGWindowListOptionIncludingWindow, window_id,
                         kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution);
    if (!image)
    {
        fprintf(stderr, "could not capture window %u\n", window_id);
        return 1;
    }

    url = CFURLCreateFromFileSystemRepresentation(NULL, (const UInt8 *)argv[2],
                                                   strlen(argv[2]), false);
    destination = url ? CGImageDestinationCreateWithURL(url, CFSTR("public.png"), 1, NULL) : NULL;
    if (!destination)
    {
        fprintf(stderr, "could not create PNG destination\n");
        if (url) CFRelease(url);
        CGImageRelease(image);
        return 1;
    }

    CGImageDestinationAddImage(destination, image, NULL);
    if (!CGImageDestinationFinalize(destination))
    {
        fprintf(stderr, "could not finalize PNG\n");
        CFRelease(destination);
        CFRelease(url);
        CGImageRelease(image);
        return 1;
    }

    CFRelease(destination);
    CFRelease(url);
    CGImageRelease(image);
    return 0;
}
