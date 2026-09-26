#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int recv_exact(SOCKET socket_fd, void *buffer, size_t length)
{
    char *cursor = buffer;
    while (length)
    {
        int received = recv(socket_fd, cursor, length > INT_MAX ? INT_MAX : (int)length, 0);
        if (received <= 0) return 0;
        cursor += received;
        length -= received;
    }
    return 1;
}

static int send_all(SOCKET socket_fd, const void *buffer, size_t length)
{
    const char *cursor = buffer;
    while (length)
    {
        int sent = send(socket_fd, cursor, length > INT_MAX ? INT_MAX : (int)length, 0);
        if (sent <= 0) return 0;
        cursor += sent;
        length -= sent;
    }
    return 1;
}

static int send_text_frame(SOCKET socket_fd, const char *text)
{
    static const unsigned char mask[4] = {0x4d, 0x61, 0x63, 0x72};
    unsigned char header[14], *payload;
    size_t length = strlen(text), header_length, i;

    header[0] = 0x81;
    if (length <= 125)
    {
        header[1] = 0x80 | (unsigned char)length;
        header_length = 2;
    }
    else if (length <= 65535)
    {
        header[1] = 0x80 | 126;
        header[2] = (unsigned char)(length >> 8);
        header[3] = (unsigned char)length;
        header_length = 4;
    }
    else
    {
        header[1] = 0x80 | 127;
        for (i = 0; i < 8; i++) header[2 + i] = (unsigned char)((uint64_t)length >> (56 - i * 8));
        header_length = 10;
    }
    memcpy(header + header_length, mask, sizeof(mask));
    header_length += sizeof(mask);

    payload = malloc(length);
    if (!payload) return 0;
    for (i = 0; i < length; i++) payload[i] = (unsigned char)text[i] ^ mask[i % 4];
    if (!send_all(socket_fd, header, header_length) || !send_all(socket_fd, payload, length))
    {
        free(payload);
        return 0;
    }
    free(payload);
    return 1;
}

static int read_text_frame(SOCKET socket_fd, FILE *output)
{
    unsigned char header[2], extended[8], mask[4] = {0};
    uint64_t length;
    unsigned char *payload;
    size_t i;
    int masked;

    if (!recv_exact(socket_fd, header, sizeof(header))) return 0;
    masked = header[1] & 0x80;
    length = header[1] & 0x7f;
    if (length == 126)
    {
        if (!recv_exact(socket_fd, extended, 2)) return 0;
        length = ((uint64_t)extended[0] << 8) | extended[1];
    }
    else if (length == 127)
    {
        if (!recv_exact(socket_fd, extended, 8)) return 0;
        length = 0;
        for (i = 0; i < 8; i++) length = (length << 8) | extended[i];
    }
    if (length > 64 * 1024 * 1024) return 0;
    if (masked && !recv_exact(socket_fd, mask, sizeof(mask))) return 0;

    payload = malloc((size_t)length + 1);
    if (!payload || !recv_exact(socket_fd, payload, (size_t)length))
    {
        free(payload);
        return 0;
    }
    if (masked)
        for (i = 0; i < length; i++) payload[i] ^= mask[i % 4];
    payload[length] = 0;

    if ((header[0] & 0x0f) == 1 || (header[0] & 0x0f) == 0)
    {
        fwrite(payload, 1, (size_t)length, output);
        fputc('\n', output);
        fflush(output);
        if (strstr((char *)payload, "\"id\":1"))
        {
            free(payload);
            return 2;
        }
    }
    free(payload);
    return 1;
}

int main(int argc, char **argv)
{
    const char *host = "127.0.0.1", *port = "8080", *path, *mode;
    const char *inspect_command =
        "{\"id\":1,\"method\":\"Runtime.evaluate\",\"params\":{\"expression\":\"JSON.stringify({title:document.title,text:document.body&&document.body.innerText,htmlLength:document.documentElement.outerHTML.length,bodyRect:document.body&&document.body.getBoundingClientRect().toJSON(),background:document.body&&getComputedStyle(document.body).backgroundColor})\",\"returnByValue\":true}}";
    const char *screenshot_command =
        "{\"id\":1,\"method\":\"Page.captureScreenshot\",\"params\":{\"format\":\"png\",\"fromSurface\":true,\"captureBeyondViewport\":false}}";
    const char *command;
    char *dynamic_command = NULL;
    struct addrinfo hints = {0}, *addresses = NULL, *address;
    WSADATA data;
    SOCKET socket_fd = INVALID_SOCKET;
    char request[2048], response[16384];
    size_t used = 0;
    int result, frame_result;
    FILE *output = stdout;

    if (argc < 3)
    {
        fprintf(stderr, "usage: cdp_probe.exe WEB_SOCKET_PATH inspect|screenshot [OUTPUT_FILE]\n"
                        "       cdp_probe.exe WEB_SOCKET_PATH eval JAVASCRIPT\n");
        return 2;
    }
    path = argv[1];
    mode = argv[2];
    if (!strcmp(mode, "inspect")) command = inspect_command;
    else if (!strcmp(mode, "screenshot")) command = screenshot_command;
    else if (!strcmp(mode, "eval"))
    {
        const char *expression;
        char *cursor;
        size_t length = 0;
        if (argc < 4)
        {
            fprintf(stderr, "eval mode requires one JavaScript argument\n");
            return 2;
        }
        expression = argv[3];
        for (const char *source = expression; *source; ++source)
            length += (*source == '\\' || *source == '"' || *source == '\n' ||
                       *source == '\r' || *source == '\t') ? 2 : 1;
        dynamic_command = malloc(length + 128);
        if (!dynamic_command) return 1;
        cursor = dynamic_command;
        cursor += sprintf(cursor,
                          "{\"id\":1,\"method\":\"Runtime.evaluate\",\"params\":{\"expression\":\"");
        for (const char *source = expression; *source; ++source)
        {
            switch (*source)
            {
            case '\\': *cursor++ = '\\'; *cursor++ = '\\'; break;
            case '"': *cursor++ = '\\'; *cursor++ = '"'; break;
            case '\n': *cursor++ = '\\'; *cursor++ = 'n'; break;
            case '\r': *cursor++ = '\\'; *cursor++ = 'r'; break;
            case '\t': *cursor++ = '\\'; *cursor++ = 't'; break;
            default: *cursor++ = *source; break;
            }
        }
        cursor += sprintf(cursor, "\",\"returnByValue\":true,\"awaitPromise\":true}}");
        command = dynamic_command;
    }
    else
    {
        fprintf(stderr, "unknown mode: %s\n", mode);
        return 2;
    }
    if (argc > 3 && strcmp(mode, "eval") && !(output = fopen(argv[3], "wb")))
    {
        fprintf(stderr, "could not open output file\n");
        return 1;
    }

    if (WSAStartup(MAKEWORD(2, 2), &data)) return 1;
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    if (getaddrinfo(host, port, &hints, &addresses)) return 1;
    for (address = addresses; address; address = address->ai_next)
    {
        socket_fd = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        if (socket_fd != INVALID_SOCKET && !connect(socket_fd, address->ai_addr, (int)address->ai_addrlen)) break;
        if (socket_fd != INVALID_SOCKET) closesocket(socket_fd);
        socket_fd = INVALID_SOCKET;
    }
    freeaddrinfo(addresses);
    if (socket_fd == INVALID_SOCKET) return 1;

    result = snprintf(request, sizeof(request),
                      "GET %s HTTP/1.1\r\nHost: %s:%s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nOrigin: http://%s:%s\r\n\r\n",
                      path, host, port, host, port);
    if (result < 0 || (size_t)result >= sizeof(request) || !send_all(socket_fd, request, result)) return 1;

    while (used + 1 < sizeof(response))
    {
        if (!recv_exact(socket_fd, response + used, 1)) return 1;
        used++;
        response[used] = 0;
        if (used >= 4 && !memcmp(response + used - 4, "\r\n\r\n", 4)) break;
    }
    if (!strstr(response, " 101 "))
    {
        fprintf(stderr, "websocket upgrade failed:\n%s\n", response);
        return 1;
    }
    if (!send_text_frame(socket_fd, command)) return 1;

    while ((frame_result = read_text_frame(socket_fd, output)) == 1) {}
    if (output != stdout) fclose(output);
    closesocket(socket_fd);
    WSACleanup();
    free(dynamic_command);
    return frame_result == 2 ? 0 : 1;
}
