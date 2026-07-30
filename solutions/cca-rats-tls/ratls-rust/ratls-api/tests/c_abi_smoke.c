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

#include <stdio.h>

static int check_init(const ratls_conf_t *conf)
{
    ratls_handle_t *handle = NULL;
    int result = ratls_init(conf, &handle);

    if (result != RATLS_SUCCESS || handle == NULL) {
        fprintf(stderr, "ratls_init failed: %s\n", ratls_last_error());
        return 1;
    }
    ratls_cleanup(handle);
    return 0;
}

int main(void)
{
    const char *version = ratls_api_version();
    if (version == NULL || version[0] == '\0')
        return 1;
    if (ratls_set_log_level(RATLS_LOG_LEVEL_NONE) != RATLS_SUCCESS)
        return 2;
    ratls_conf_t defaults;
    if (ratls_conf_init(&defaults) != RATLS_SUCCESS)
        return 3;
    if (defaults.attester_type != RATLS_ATTESTER_CCA ||
        defaults.verifier_type != RATLS_VERIFIER_CCA ||
        defaults.cert_algo != RATLS_CERT_ALGO_DEFAULT ||
        defaults.struct_size != sizeof(ratls_conf_t) ||
        defaults.server != 0 ||
        defaults.mutual != 0 ||
        defaults.tls_verify.verify_peer_certificate != 0)
        return 4;
    if (check_init(NULL) != 0)
        return 5;

    const ratls_conf_t client = {
        .struct_size = sizeof(ratls_conf_t),
        .attester_type = RATLS_ATTESTER_NONE,
        .verifier_type = RATLS_VERIFIER_CCA,
        .tls_type = "openssl",
        .crypto_type = "openssl",
        .cert_algo = RATLS_CERT_ALGO_RSA_3072_SHA256,
        .mutual = 0,
        .server = 0,
        .custom_claims = NULL,
        .custom_claims_len = 0,
    };
    if (check_init(&client) != 0)
        return 6;

    const ratls_conf_t server = {
        .struct_size = sizeof(ratls_conf_t),
        .attester_type = RATLS_ATTESTER_CCA,
        .verifier_type = RATLS_VERIFIER_NONE,
        .tls_type = NULL,
        .crypto_type = NULL,
        .cert_algo = RATLS_CERT_ALGO_ECC_256_SHA256,
        .mutual = 0,
        .server = 1,
        .custom_claims = NULL,
        .custom_claims_len = 0,
    };
    if (check_init(&server) != 0)
        return 7;

    ratls_handle_t *invalid = NULL;
    const ratls_conf_t invalid_client = {
        .struct_size = sizeof(ratls_conf_t),
        .attester_type = RATLS_ATTESTER_NONE,
        .verifier_type = RATLS_VERIFIER_NONE,
        .cert_algo = RATLS_CERT_ALGO_DEFAULT,
    };
    if (ratls_init(&invalid_client, &invalid) != RATLS_INIT_ERROR || invalid != NULL)
        return 8;

    if (ratls_conf_init(NULL) != RATLS_INVALID_ARGUMENT)
        return 9;

    ratls_conf_t no_trust;
    if (ratls_conf_init(&no_trust) != RATLS_SUCCESS)
        return 10;
    no_trust.attester_type = RATLS_ATTESTER_NONE;
    no_trust.tls_verify.verify_peer_certificate = 1;
    if (ratls_init(&no_trust, &invalid) != RATLS_INIT_ERROR || invalid != NULL)
        return 11;

    ratls_conf_t wrong_size;
    if (ratls_conf_init(&wrong_size) != RATLS_SUCCESS)
        return 12;
    wrong_size.struct_size = 0;
    if (ratls_init(&wrong_size, &invalid) != RATLS_INIT_ERROR || invalid != NULL)
        return 13;

    ratls_conf_t oversized;
    if (ratls_conf_init(&oversized) != RATLS_SUCCESS)
        return 14;
    oversized.certificate.issuer_private_key.data = (const uint8_t *)(uintptr_t)1;
    oversized.certificate.issuer_private_key.len =
        RATLS_MAX_ISSUER_PRIVATE_KEY_PEM_SIZE + 1;
    oversized.certificate.issuer_certificate_chain.data = (const uint8_t *)"x";
    oversized.certificate.issuer_certificate_chain.len = 1;
    if (ratls_init(&oversized, &invalid) != RATLS_INIT_ERROR || invalid != NULL)
        return 15;

    ratls_conf_t ignored_tls;
    if (ratls_conf_init(&ignored_tls) != RATLS_SUCCESS)
        return 16;
    ignored_tls.tls_verify.verify_peer_certificate = 0;
    ignored_tls.tls_verify.trusted_ca_chain.data = (const uint8_t *)(uintptr_t)1;
    ignored_tls.tls_verify.trusted_ca_chain.len = SIZE_MAX;
    ignored_tls.tls_verify.expected_peer_name = (const char *)(uintptr_t)1;
    if (check_init(&ignored_tls) != 0)
        return 17;

    return 0;
}
