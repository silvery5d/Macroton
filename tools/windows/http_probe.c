#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    const char *host = argc > 1 ? argv[1] : "127.0.0.1";
    const char *port = argc > 2 ? argv[2] : "8080";
    const char *path = argc > 3 ? argv[3] : "/json";
    struct addrinfo hints = {0}, *addresses = NULL, *address;
    WSADATA data;
    SOCKET socket_fd = INVALID_SOCKET;
    char request[1024], buffer[4096];
    DWORD timeout_ms = 5000;
    int result, received;
    size_t total_received = 0;

    if (WSAStartup(MAKEWORD(2, 2), &data))
    {
        fprintf(stderr, "WSAStartup failed\n");
        return 1;
    }

    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    result = getaddrinfo(host, port, &hints, &addresses);
    if (result)
    {
        fprintf(stderr, "getaddrinfo failed: %d\n", result);
        WSACleanup();
        return 1;
    }

    for (address = addresses; address; address = address->ai_next)
    {
        socket_fd = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        if (socket_fd == INVALID_SOCKET) continue;
        if (!connect(socket_fd, address->ai_addr, (int)address->ai_addrlen)) break;
        closesocket(socket_fd);
        socket_fd = INVALID_SOCKET;
    }
    freeaddrinfo(addresses);

    if (socket_fd == INVALID_SOCKET)
    {
        fprintf(stderr, "connect failed: %d\n", WSAGetLastError());
        WSACleanup();
        return 1;
    }

    fprintf(stderr, "connected to %s:%s\n", host, port);
    setsockopt(socket_fd, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout_ms, sizeof(timeout_ms));
    setsockopt(socket_fd, SOL_SOCKET, SO_SNDTIMEO, (const char *)&timeout_ms, sizeof(timeout_ms));

    result = snprintf(request, sizeof(request),
                      "GET %s HTTP/1.1\r\nHost: %s:%s\r\nConnection: close\r\n\r\n",
                      path, host, port);
    if (result < 0 || (size_t)result >= sizeof(request) ||
        send(socket_fd, request, result, 0) != result)
    {
        fprintf(stderr, "send failed: %d\n", WSAGetLastError());
        closesocket(socket_fd);
        WSACleanup();
        return 1;
    }
    fprintf(stderr, "request sent\n");

    while ((received = recv(socket_fd, buffer, sizeof(buffer), 0)) > 0)
    {
        fwrite(buffer, 1, received, stdout);
        total_received += received;
    }

    if (received == SOCKET_ERROR && (!total_received || WSAGetLastError() != WSAETIMEDOUT))
    {
        fprintf(stderr, "recv failed: %d\n", WSAGetLastError());
        closesocket(socket_fd);
        WSACleanup();
        return 1;
    }

    closesocket(socket_fd);
    WSACleanup();
    return 0;
}
