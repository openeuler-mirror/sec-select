#ifndef TPM_DRIVER_OPS_H
#define TPM_DRIVER_OPS_H

#include <linux/types.h>

#define SPI_CMD_LEN 0x4U

uint32_t read_tpm_driver(uintptr_t reg, uint8_t *ch);
uint32_t write_tpm_driver(uintptr_t reg, uint8_t ch);

#endif
