/*
 * Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
 * Global Trust Authority is licensed under the Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at:
 *     http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND, EITHER EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR
 * PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

#include "ratls_api.h"

#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static int check_ratls(int rc, const char *operation)
{
    if (rc == RATLS_SUCCESS)
        return 1;
    fprintf(stderr, "%s 失败（%d）：%s\n", operation, rc, ratls_last_error());
    return 0;
}

static int listen_tcp(uint16_t port)
{
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    int enabled = 1;
    if (fd < 0)
        return -1;
    (void)setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &enabled, sizeof(enabled));

    struct sockaddr_in address = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(fd, (const struct sockaddr *)&address, sizeof(address)) != 0 ||
        listen(fd, 16) != 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static int send_all(ratls_handle_t *handle, const uint8_t *data, size_t len)
{
    size_t offset = 0;
    while (offset < len) {
        size_t written = 0;
        if (!check_ratls(ratls_transmit(handle, data + offset, len - offset, &written),
                         "ratls_transmit") ||
            written == 0)
            return 0;
        offset += written;
    }
    return 1;
}

int main(int argc, char **argv)
{
    uint16_t port = argc > 1 ? (uint16_t)strtoul(argv[1], NULL, 10) : 1234;
    uint8_t request[1024];
    size_t request_len = 0;
    ratls_handle_t *handle = NULL;

    int listener = listen_tcp(port);
    if (listener < 0) {
        fprintf(stderr, "TCP 监听失败：%s\n", strerror(errno));
        return 1;
    }
    printf("监听 0.0.0.0:%u\n", port);

    int client_fd = accept(listener, NULL, NULL);
    if (client_fd < 0) {
        fprintf(stderr, "accept 失败：%s\n", strerror(errno));
        close(listener);
        return 1;
    }

    ratls_conf_t conf;
    if (!check_ratls(ratls_conf_init(&conf), "ratls_conf_init"))
        goto failed;
    conf.server = 1;
    conf.mutual = 0;
    conf.attester_type = RATLS_ATTESTER_CCA;  /* 服务端从 Linux TSM 采集 CCA evidence。 */
    conf.verifier_type = RATLS_VERIFIER_NONE; /* 单向证明时服务端不验证客户端。 */

    /*
     * 如需 CA 动态签发本端证书，可设置：
     *
     * conf.certificate.issuer_private_key =
     *     (ratls_buffer_t){issuer_key_pem, issuer_key_pem_len};
     * conf.certificate.issuer_certificate_chain =
     *     (ratls_buffer_t){issuer_chain_pem, issuer_chain_pem_len};
     */

    if (!check_ratls(ratls_init(&conf, &handle), "ratls_init") ||
        !check_ratls(ratls_negotiate_fd(handle, client_fd), "ratls_negotiate_fd") ||
        !check_ratls(ratls_receive(handle, request, sizeof(request), &request_len),
                     "ratls_receive"))
        goto failed;

    printf("收到客户端消息: %zu 字节\n", request_len);
    if (!send_all(handle, request, request_len))
        goto failed;

    ratls_cleanup(handle);
    close(client_fd);
    close(listener);
    return 0;

failed:
    ratls_cleanup(handle);
    close(client_fd);
    close(listener);
    return 1;
}
