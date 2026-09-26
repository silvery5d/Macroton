#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <stdio.h>

int main(int argc, char **argv) {
    WSADATA data;
    struct addrinfo hints = {0};
    struct addrinfo *result = NULL;
    struct addrinfo *entry;
    char address[INET6_ADDRSTRLEN];
    int status;

    if (argc != 2) {
        fprintf(stderr, "usage: network_probe HOSTNAME\n");
        return 2;
    }
    if ((status = WSAStartup(MAKEWORD(2, 2), &data)) != 0) {
        fprintf(stderr, "WSAStartup failed: %d\n", status);
        return 1;
    }
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if ((status = getaddrinfo(argv[1], "443", &hints, &result)) != 0) {
        fprintf(stderr, "getaddrinfo failed: %d\n", status);
        WSACleanup();
        return 1;
    }
    for (entry = result; entry != NULL; entry = entry->ai_next) {
        void *source;
        if (entry->ai_family == AF_INET) {
            source = &((struct sockaddr_in *)entry->ai_addr)->sin_addr;
        } else if (entry->ai_family == AF_INET6) {
            source = &((struct sockaddr_in6 *)entry->ai_addr)->sin6_addr;
        } else {
            continue;
        }
        if (InetNtopA(entry->ai_family, source, address, sizeof(address))) {
            printf("family=%d address=%s\n", entry->ai_family, address);
        }
    }
    freeaddrinfo(result);
    WSACleanup();
    return 0;
}
