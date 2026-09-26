#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int send_all(SOCKET socket_handle, const char *bytes, int length) {
    int sent = 0;
    while (sent < length) {
        int count = send(socket_handle, bytes + sent, length - sent, 0);
        if (count <= 0) return 0;
        sent += count;
    }
    return 1;
}

static int receive_http_headers(SOCKET socket_handle) {
    char buffer[8192];
    int used = 0;
    while (used < (int)sizeof(buffer) - 1) {
        int count = recv(socket_handle, buffer + used, sizeof(buffer) - 1 - used, 0);
        if (count <= 0) return 0;
        used += count;
        buffer[used] = '\0';
        if (strstr(buffer, "\r\n\r\n")) {
            if (!strstr(buffer, " 101 ")) {
                fputs("CDP WebSocket upgrade failed\n", stderr);
                return 0;
            }
            return 1;
        }
    }
    return 0;
}

static int send_text_frame(SOCKET socket_handle, const char *text) {
    size_t length = strlen(text);
    unsigned char header[14];
    int header_length = 0;
    const unsigned char mask[4] = {0x4d, 0x61, 0x63, 0x72};
    header[header_length++] = 0x81;
    if (length < 126) {
        header[header_length++] = (unsigned char)(0x80 | length);
    } else if (length <= 0xffff) {
        header[header_length++] = 0x80 | 126;
        header[header_length++] = (unsigned char)(length >> 8);
        header[header_length++] = (unsigned char)length;
    } else {
        fputs("CDP request is too large\n", stderr);
        return 0;
    }
    memcpy(header + header_length, mask, sizeof(mask));
    header_length += sizeof(mask);
    char *masked = malloc(length);
    if (!masked) return 0;
    for (size_t index = 0; index < length; index++) {
        masked[index] = text[index] ^ mask[index % 4];
    }
    int ok = send_all(socket_handle, (const char *)header, header_length)
        && send_all(socket_handle, masked, (int)length);
    free(masked);
    return ok;
}

static int receive_exact(SOCKET socket_handle, unsigned char *bytes, size_t length) {
    size_t received = 0;
    while (received < length) {
        int count = recv(socket_handle, (char *)bytes + received,
                         (int)(length - received), 0);
        if (count <= 0) return 0;
        received += (size_t)count;
    }
    return 1;
}

static int receive_result(SOCKET socket_handle) {
    for (int frame = 0; frame < 64; frame++) {
        unsigned char header[2];
        if (!receive_exact(socket_handle, header, sizeof(header))) return 0;
        unsigned long long length = header[1] & 0x7f;
        if (length == 126) {
            unsigned char extended[2];
            if (!receive_exact(socket_handle, extended, sizeof(extended))) return 0;
            length = ((unsigned long long)extended[0] << 8) | extended[1];
        } else if (length == 127) {
            unsigned char extended[8];
            if (!receive_exact(socket_handle, extended, sizeof(extended))) return 0;
            length = 0;
            for (int index = 0; index < 8; index++) {
                length = (length << 8) | extended[index];
            }
        }
        if (length > 4 * 1024 * 1024) {
            fputs("CDP response is too large\n", stderr);
            return 0;
        }
        char *payload = malloc((size_t)length + 1);
        if (!payload) return 0;
        if (!receive_exact(socket_handle, (unsigned char *)payload, (size_t)length)) {
            free(payload);
            return 0;
        }
        payload[length] = '\0';
        int opcode = header[0] & 0x0f;
        if (opcode == 1 && strstr(payload, "\"id\":1")) {
            puts(payload);
            free(payload);
            return 1;
        }
        free(payload);
    }
    fputs("CDP response did not contain command id 1\n", stderr);
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 3 || (strcmp(argv[2], "click-install") != 0 &&
                      strcmp(argv[2], "dismiss-interstitial") != 0)) {
        fputs("usage: win32_steam_cdp_action.exe WEB_SOCKET_PATH click-install|dismiss-interstitial\n", stderr);
        return 2;
    }
    const char *expression = strcmp(argv[2], "click-install") == 0
        ? "(()=>{const c=[...document.querySelectorAll('button,[role=button]')]"
          ".filter(e=>(e.innerText||e.textContent||'').trim()==='Install');"
          "if(c.length!==1)return JSON.stringify({clicked:false,matches:c.length});"
          "c[0].click();return JSON.stringify({clicked:true,matches:1});})()"
        : "(()=>{const c=[...document.querySelectorAll('button,[role=button]')]"
          ".filter(e=>/^(continue|ok|close|got it|i understand)$/i.test((e.innerText||e.textContent||'').trim()));"
          "if(!c.length)return JSON.stringify({clicked:false,matches:0});"
          "c[0].click();return JSON.stringify({clicked:true,matches:c.length,text:(c[0].innerText||c[0].textContent||'').trim()});})()";
    char request_json[2048];
    snprintf(request_json, sizeof(request_json),
             "{\"id\":1,\"method\":\"Runtime.evaluate\",\"params\":{"
             "\"expression\":\"%s\",\"returnByValue\":true}}", expression);

    WSADATA winsock;
    if (WSAStartup(MAKEWORD(2, 2), &winsock) != 0) return 1;
    SOCKET socket_handle = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (socket_handle == INVALID_SOCKET) return 1;
    struct sockaddr_in address;
    memset(&address, 0, sizeof(address));
    address.sin_family = AF_INET;
    address.sin_port = htons(8080);
    inet_pton(AF_INET, "127.0.0.1", &address.sin_addr);
    if (connect(socket_handle, (struct sockaddr *)&address, sizeof(address)) != 0) {
        fputs("cannot connect to Steam CEF on 127.0.0.1:8080\n", stderr);
        closesocket(socket_handle);
        WSACleanup();
        return 1;
    }
    char upgrade[2048];
    snprintf(upgrade, sizeof(upgrade),
             "GET %s HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nUpgrade: websocket\r\n"
             "Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
             "Sec-WebSocket-Version: 13\r\nOrigin: http://127.0.0.1:8080\r\n\r\n",
             argv[1]);
    int ok = send_all(socket_handle, upgrade, (int)strlen(upgrade))
        && receive_http_headers(socket_handle)
        && send_text_frame(socket_handle, request_json)
        && receive_result(socket_handle);
    closesocket(socket_handle);
    WSACleanup();
    return ok ? 0 : 1;
}
