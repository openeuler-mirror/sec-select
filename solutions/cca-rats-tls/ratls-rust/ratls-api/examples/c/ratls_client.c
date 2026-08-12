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

/* 打印内置密码学校验通过后的 CCA claims，并执行应用自己的策略判断。 */
static int verify_evidence(const ratls_verified_evidence_t *evidence, void *user_data)
{
    (void)user_data;
    printf("对端 evidence 类型: %s\n", evidence->evidence_type);
    printf("对端 CCA claims: %.*s\n",
           (int)evidence->claims_json.len,
           (const char *)evidence->claims_json.data);

    /*
     * 生产环境应在这里检查 RIM、TCB、平台生命周期、软件组件或 custom claims。
     * 返回 1 表示接受，返回 0 会使 ratls_negotiate_fd() 失败。
     */
    return 1;
}

static int check_ratls(int rc, const char *operation)
{
    if (rc == RATLS_SUCCESS)
        return 1;
    fprintf(stderr, "%s 失败（%d）：%s\n", operation, rc, ratls_last_error());
    return 0;
}

static int connect_tcp(const char *ip, uint16_t port)
{
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0)
        return -1;

    struct sockaddr_in address = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
    };
    if (inet_pton(AF_INET, ip, &address.sin_addr) != 1 ||
        connect(fd, (const struct sockaddr *)&address, sizeof(address)) != 0) {
        close(fd);
        return -1;
    }
    return fd;
}

/* ratls_transmit() 允许短写，因此完整消息需要循环发送。 */
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
    const char *ip = argc > 1 ? argv[1] : "127.0.0.1";
    uint16_t port = argc > 2 ? (uint16_t)strtoul(argv[2], NULL, 10) : 1234;
    const char message[] = "hello from C";
    uint8_t response[1024];
    size_t response_len = 0;
    ratls_handle_t *handle = NULL;

    int fd = connect_tcp(ip, port);
    if (fd < 0) {
        fprintf(stderr, "TCP 连接失败：%s\n", strerror(errno));
        return 1;
    }

    ratls_conf_t conf;
    if (!check_ratls(ratls_conf_init(&conf), "ratls_conf_init"))
        goto failed;
    conf.server = 0;
    conf.mutual = 0;
    conf.attester_type = RATLS_ATTESTER_NONE; /* 单向证明时客户端不生成 evidence。 */
    conf.verifier_type = RATLS_VERIFIER_CCA;  /* 客户端验证服务端 CCA evidence。 */

    /*
     * 如需标准 TLS CA 校验，可设置：
     *
     * conf.tls_verify.verify_peer_certificate = 1;
     * conf.tls_verify.use_system_ca = 1;
     * conf.tls_verify.trusted_ca_chain = (ratls_buffer_t){ca_pem, ca_pem_len};
     * conf.tls_verify.expected_peer_name = "server.example.com";
     */

    if (!check_ratls(ratls_init(&conf, &handle), "ratls_init") ||
        !check_ratls(ratls_set_verification_callback(handle, verify_evidence, NULL),
                     "ratls_set_verification_callback") ||
        !check_ratls(ratls_negotiate_fd(handle, fd), "ratls_negotiate_fd") ||
        !send_all(handle, (const uint8_t *)message, strlen(message)) ||
        !check_ratls(ratls_receive(handle, response, sizeof(response), &response_len),
                     "ratls_receive"))
        goto failed;

    printf("收到服务端回复: %zu 字节\n", response_len);
    ratls_cleanup(handle);
    close(fd);
    return 0;

failed:
    ratls_cleanup(handle);
    close(fd);
    return 1;
}
