#ifndef TPM_DRIVER_COMMON_H
#define TPM_DRIVER_COMMON_H

#include <linux/types.h>

#define TEE_OK 0
#define TEE_ERR_BUSY 1
#define TEE_ERR_BAD_STATE 2
#define TEE_ERR_BAD_PARAM 3
#define TEE_ERR_OUT_OF_MEMORY 4
#define TEE_ERR_NOT_SUPPORTED 5
#define TEE_ERR_SHORT_BUFFER 6

extern void tpm_delay_with_us(unsigned long usec);

#endif
