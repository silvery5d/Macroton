#include <ApplicationServices/ApplicationServices.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>

static int parse_coordinate(const char *text, double *value)
{
    char *end = NULL;
    errno = 0;
    *value = strtod(text, &end);
    return errno == 0 && end && *end == '\0' && *value >= 0.0;
}

int main(int argc, char **argv)
{
    CGEventRef move, down, up;
    CGPoint point;

    if (argc != 4 || strcmp(argv[1], "--click") != 0 ||
        !parse_coordinate(argv[2], &point.x) ||
        !parse_coordinate(argv[3], &point.y)) {
        fprintf(stderr, "usage: input_probe --click SCREEN_X SCREEN_Y\n");
        return 2;
    }

    move = CGEventCreateMouseEvent(NULL, kCGEventMouseMoved, point,
                                   kCGMouseButtonLeft);
    down = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseDown, point,
                                   kCGMouseButtonLeft);
    up = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseUp, point,
                                 kCGMouseButtonLeft);
    if (!move || !down || !up) {
        fprintf(stderr, "could not create mouse events\n");
        if (move) CFRelease(move);
        if (down) CFRelease(down);
        if (up) CFRelease(up);
        return 1;
    }

    CGEventPost(kCGHIDEventTap, move);
    usleep(150000);
    CGEventPost(kCGHIDEventTap, down);
    usleep(150000);
    CGEventPost(kCGHIDEventTap, up);

    CFRelease(move);
    CFRelease(down);
    CFRelease(up);
    return 0;
}
