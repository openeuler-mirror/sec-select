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

#ifndef RATLS_API_H
#define RATLS_API_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 返回值：0 表示成功，负数表示错误。失败后立即调用 ratls_last_error() 获取原因。 */
#define RATLS_SUCCESS 0
#define RATLS_INVALID_ARGUMENT -1
#define RATLS_INIT_ERROR -2
#define RATLS_NEGOTIATE_ERROR -3
#define RATLS_TRANSMIT_ERROR -4
#define RATLS_RECEIVE_ERROR -5
#define RATLS_SYSTEM_ERROR -6
#define RATLS_PANIC -7

/* RA-TLS 证书密钥算法。推荐使用 RATLS_CERT_ALGO_DEFAULT。 */
#define RATLS_CERT_ALGO_RSA_3072_SHA256 0
#define RATLS_CERT_ALGO_ECC_256_SHA256 1
/* 仅作为枚举边界，不可作为配置值。 */
#define RATLS_CERT_ALGO_MAX 2
#define RATLS_CERT_ALGO_DEFAULT 3

/* 全局日志级别。 */
#define RATLS_LOG_LEVEL_DEBUG 0
#define RATLS_LOG_LEVEL_INFO 1
#define RATLS_LOG_LEVEL_WARN 2
#define RATLS_LOG_LEVEL_ERROR 3
#define RATLS_LOG_LEVEL_FATAL 4
#define RATLS_LOG_LEVEL_NONE 5

/* ratls_init() 接受的输入上限；超出时返回 RATLS_INIT_ERROR。 */
#define RATLS_MAX_ISSUER_PRIVATE_KEY_PEM_SIZE ((size_t)65536)
#define RATLS_MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE ((size_t)1048576)
#define RATLS_MAX_TRUSTED_CA_CHAIN_PEM_SIZE ((size_t)4194304)
#define RATLS_MAX_SUBJECT_ALT_NAMES ((size_t)64)
#define RATLS_MAX_SUBJECT_ALT_NAME_LENGTH ((size_t)2048)
#define RATLS_MAX_EXPECTED_PEER_NAME_LENGTH ((size_t)253)
#define RATLS_MAX_CUSTOM_CLAIMS ((size_t)64)
#define RATLS_MAX_CUSTOM_CLAIM_NAME_LENGTH ((size_t)128)
#define RATLS_MAX_CUSTOM_CLAIM_VALUE_LENGTH ((size_t)65536)
#define RATLS_MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH ((size_t)262144)

/* 本地证明生成器。服务端以及双向证明的客户端必须配置 attester。 */
#define RATLS_ATTESTER_NONE 0
#define RATLS_ATTESTER_CCA 1

/* 对端证明验证器。客户端以及双向证明的服务端必须配置 verifier。 */
#define RATLS_VERIFIER_NONE 0
#define RATLS_VERIFIER_CCA 1

/* 不透明会话句柄。只能由本库创建和释放，调用方不得访问其内部字段。 */
typedef struct ratls_handle ratls_handle_t;

/* 借用的二进制数据视图，不保证以 '\0' 结尾。 */
typedef struct ratls_buffer {
    const uint8_t *data;
    size_t len;
} ratls_buffer_t;

/**
 * 本端动态证书签发配置。
 *
 * issuer_private_key 和 issuer_certificate_chain 都为空时，库自动生成叶子私钥并
 * 动态自签证书；两者都非空时，库生成临时叶子私钥，并使用 CA 私钥动态签发证书。
 * 只配置其中一项属于无效配置。
 */
typedef struct ratls_certificate_conf {
    /* 未加密的 CA 私钥 PEM 数据；不支持加密私钥。 */
    ratls_buffer_t issuer_private_key;
    /* 与 CA 私钥匹配的 PEM 证书链；证书顺序不限，Root CA 可包含在内。 */
    ratls_buffer_t issuer_certificate_chain;
    /* 可选 SAN 字符串数组，元素格式为 DNS:、IP: 或 URI:。 */
    const char *const *subject_alt_names;
    size_t subject_alt_names_len;
} ratls_certificate_conf_t;

/**
 * 标准 TLS 对端证书校验配置。
 *
 * verify_peer_certificate 为 0 时，不校验 CA 信任根和对端名称，但仍校验证书签名、
 * 有效期、约束、KeyUsage、角色 EKU，并继续使用现有 RA 校验；其他字段全部忽略。
 * 开启后，额外执行 CA 信任链和可选名称校验，且 RA 校验也必须通过。
 * 本配置不启用 CRL 或 OCSP 吊销检查；加载系统 CA 也只表示加载系统信任根。
 */
typedef struct ratls_tls_verify_conf {
    /* 非 0 启用标准 TLS 对端证书校验。 */
    uint8_t verify_peer_certificate;
    /* 非 0 加载 OpenSSL 默认系统 CA，并与 trusted_ca_chain 合并。 */
    uint8_t use_system_ca;
    /* 可选的对端自定义可信 CA PEM bundle。 */
    ratls_buffer_t trusted_ca_chain;
    /* 可选 DNS 名称或 IP；NULL/空字符串表示不校验名称。 */
    const char *expected_peer_name;
} ratls_tls_verify_conf_t;

/**
 * RATS-TLS 配置。
 *
 * 推荐先调用 ratls_conf_init() 写入安全默认值，再按需修改字段。传给
 * ratls_init() 后，配置中的字符串和 custom claims 会被复制，调用方随后可以释放。
 */
typedef struct ratls_conf {
    /* 结构体尺寸；必须由 ratls_conf_init() 初始化。 */
    size_t struct_size;
    /* 本地 evidence 生成器，取值为 RATLS_ATTESTER_*。 */
    uint32_t attester_type;
    /* 对端 evidence 验证器，取值为 RATLS_VERIFIER_*。 */
    uint32_t verifier_type;
    /* TLS 后端名称；NULL 使用默认值。目前支持 "openssl"。 */
    const char *tls_type;
    /* 密码学后端名称；NULL 使用默认值。目前支持 "openssl"。 */
    const char *crypto_type;
    /* RA-TLS 证书密钥算法，取值为 RATLS_CERT_ALGO_*。 */
    uint32_t cert_algo;
    /* 非 0 启用双向证明；0 表示只验证服务端。 */
    uint8_t mutual;
    /* 非 0 表示服务端；0 表示客户端。 */
    uint8_t server;
    /* 要写入本地 evidence 的应用自定义 claims；不使用时为 NULL。 */
    const struct ratls_custom_claim *custom_claims;
    /* custom_claims 数组元素个数。 */
    size_t custom_claims_len;
    /* 本端动态证书签发配置，客户端和服务端共用。 */
    ratls_certificate_conf_t certificate;
    /* 标准 TLS 对端证书校验配置，客户端和服务端共用。 */
    ratls_tls_verify_conf_t tls_verify;
} ratls_conf_t;

/* 自定义 claim。name 必须是 UTF-8、以 '\0' 结尾的字符串。 */
typedef struct ratls_custom_claim {
    const char *name;
    ratls_buffer_t value;
} ratls_custom_claim_t;

/**
 * 已通过内置密码学校验的对端 evidence。
 *
 * 结构体及其所有指针仅在 verification callback 执行期间有效。如需在回调外使用，
 * 调用方必须自行复制。raw_token 和 claims_json 都不保证以 '\0' 结尾。
 */
typedef struct ratls_verified_evidence {
    /* evidence 类型，以 '\0' 结尾；当前为 "cca"。 */
    const char *evidence_type;
    /* 原始 CCA token。 */
    ratls_buffer_t raw_token;
    /* UTF-8 JSON 格式的公开 CCA claims，不带结尾 '\0'。 */
    ratls_buffer_t claims_json;
    /* 对端随 evidence 携带的应用自定义 claims。 */
    const ratls_custom_claim_t *custom_claims;
    size_t custom_claims_len;
} ratls_verified_evidence_t;

/**
 * 应用策略回调。
 * 返回非 0 接受对端；返回 0 拒绝连接。不得保存 evidence 中的借用指针。
 */
typedef int (*ratls_verification_callback_t)(const ratls_verified_evidence_t *evidence,
                                             void *user_data);

/* 返回 ABI 版本字符串。返回指针由库持有，调用方不得释放。 */
const char *ratls_api_version(void);

/**
 * 返回当前线程最近一次 FFI 调用的错误字符串。
 * 指针由库持有，同一线程下一次更新错误状态后失效。
 */
const char *ratls_last_error(void);

/* 设置进程级日志级别。 */
int ratls_set_log_level(uint32_t level);

/**
 * 使用库的默认值初始化配置。
 * 默认值：CCA attester/verifier、OpenSSL、ECC P-256、客户端、单向证明、
 * 动态自签证书、关闭标准 TLS CA 校验。
 */
int ratls_conf_init(ratls_conf_t *conf);

/**
 * 创建会话句柄。成功时 *handle_out 非 NULL，最终必须调用 ratls_cleanup()。
 * conf 可为 NULL，此时直接使用默认配置；推荐显式调用 ratls_conf_init()。
 */
int ratls_init(const ratls_conf_t *conf, ratls_handle_t **handle_out);

/* 关闭 TLS 流并释放句柄。handle 可为 NULL；同一句柄只能释放一次。 */
void ratls_cleanup(ratls_handle_t *handle);

/* 注册应用层 evidence 策略回调；callback 为 NULL 时取消回调。 */
int ratls_set_verification_callback(ratls_handle_t *handle,
                                    ratls_verification_callback_t callback,
                                    void *user_data);

/**
 * 在已经 connect()/accept() 成功的 TCP socket 上执行 RATS-TLS 握手。
 * 库会 dup(fd)，因此原 fd 仍由调用方负责 close()。成功后才能调用收发接口。
 */
int ratls_negotiate_fd(ratls_handle_t *handle, int fd);

/**
 * 发送 TLS 应用数据。written_out 可为 NULL。
 * 成功不保证一次写完 data_len 字节，调用方应根据 written_out 循环发送。
 */
int ratls_transmit(ratls_handle_t *handle,
                   const uint8_t *data,
                   size_t data_len,
                   size_t *written_out);

/**
 * 接收 TLS 应用数据。read_out 可为 NULL；成功且读取长度为 0 表示对端已关闭连接。
 * 本接口是字节流，不保留消息边界，应用应自行设计长度头或其他 framing。
 */
int ratls_receive(ratls_handle_t *handle,
                  uint8_t *buffer,
                  size_t buffer_len,
                  size_t *read_out);

#ifdef __cplusplus
}
#endif

#endif /* RATLS_API_H */
