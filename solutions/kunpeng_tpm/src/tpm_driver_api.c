#include <linux/types.h>
#include "tpm_driver_util.h"
#include "tpm_driver_api.h"
#include "tpm_driver_cmd.h"
#include "tpm_driver_ops.h"
#include "spi_drv.h"
#include "tpm_driver_gpio.h"
#include "tpm_driver_spi_reg_v1.h"
#include "tpm_driver_v2_interface.h"

uint32_t check_tpm_status(void)
{
	uint8_t id_buf[BYTE_4] = {0};
	uint32_t tpm_id = 0;
	uint32_t ret;

	ret = read_tpm_driver(TPM_ID_ADDR_0, &id_buf[0]);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm id read failed, 0x%x\n", ret);
		return TEE_ERR_NOT_SUPPORTED;
	}

	tpm_id = (uint32_t)id_buf[0];
	printk(KERN_INFO "check tpm id: 0x%x\n", tpm_id);

	return (tpm_id != 0) ? TEE_OK : TEE_ERR_NOT_SUPPORTED;
}
