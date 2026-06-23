#ifndef TPM_DRIVER_CMD_H
#define TPM_DRIVER_CMD_H

#include "tpm_driver_common.h"

#define TPM_RECV_CMD_LEN 4096U
#define TPM_SEND_CMD_LEN 4096U

uint32_t tpm_cmd_transmit(uint8_t *send_data, uint32_t send_len, uint8_t *recv_data, uint32_t *recv_len);

#endif
