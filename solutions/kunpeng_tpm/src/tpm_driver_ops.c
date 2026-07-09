#include "tpm_driver_util.h"
#include "tpm_driver_v2_interface.h"
#include "tpm_driver_gpio.h"
#include "tpm_driver_ops.h"
#include "tpm_driver_common.h"
#include "spi_drv.h"

static uint32_t tpm_spi_send_cmd(uint32_t bus, uint32_t dev, uint8_t *tx_buf,
		uint32_t tx_size, uint8_t *rx_buf, uint32_t rx_size)
{
	NO_USE_PARAM(tx_size);
	uint32_t ret;
	uint32_t try_times = TPM_ACK_TRY_TIMES;

	ret = spi_data_ctrl(bus, dev, tx_buf, SPI_CMD_LEN, rx_buf, rx_size, 0);
	if (ret != TEE_OK) {
		printk(KERN_INFO "read or write spi failed 0x%x\n", ret);
		return ret;
	}

	if (rx_buf[ARR_IND_3] == 0x01U) {
		return TEE_OK;
	}

	while (try_times != 0) {
		rx_buf[0] = 0;
		(void)spi_data_ctrl(bus, dev, tx_buf, 0, rx_buf, rx_size, 1);
		if (rx_buf[0] == 0x01U) {
			return TEE_OK;
		}
		tpm_delay_with_us(TPM_SPI_DELAY);
		try_times--;
	}

	printk(KERN_INFO "tpm spi send cmd err\n");
	return TEE_ERR_BAD_STATE;
}

static uint32_t tpm_spi_read(uint32_t bus, uint32_t dev, uintptr_t addr, uint32_t len, void *buf)
{
	uint32_t ret;
	uint8_t tx_buf[SPI_CMD_LEN + MAX_SPI_FRAMESIZE] = {0};
	uint8_t rx_buf[SPI_CMD_LEN + MAX_SPI_FRAMESIZE] = {0};

	if ((len > MAX_SPI_FRAMESIZE) || (len == 0) || (buf == NULL)) {
		printk(KERN_INFO "tpm spi read param err, len:0x%x\n", len);
		return TEE_ERR_BAD_PARAM;
	}
	
	tx_buf[0] = 0x80U | ((uint8_t)len - 1);
	tx_buf[ARR_IND_1] = (addr >> BIT_SHIFT_16) & 0xFFU;
	tx_buf[ARR_IND_2] = (addr >> BIT_SHIFT_8) & 0xFFU;
	tx_buf[ARR_IND_3] = (addr) & 0xFFU;

	ioconfig_gpio(dev, bus);
	spi_cs_enable(dev);

	ret = tpm_spi_send_cmd(bus, dev, tx_buf, sizeof(tx_buf), rx_buf, sizeof(rx_buf));
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm spi read cmd err, 0x%x\n", ret);
		goto spi_release;
	}

	(void)memset(rx_buf, 0, sizeof(rx_buf));
	ret = spi_data_ctrl(bus, dev, tx_buf, 0, rx_buf, sizeof(rx_buf), 1);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm spi read data err, 0x%x\n", ret);
		goto spi_release;
	}

	(void)memcpy(buf, (void *)rx_buf, sizeof(uint8_t));

	ret = TEE_OK;
spi_release:
	spi_cs_enable(dev);
	ioconfig_spi(dev, bus);

	return ret;
}

static uint32_t tpm_spi_write(uint32_t bus, uint32_t dev, uintptr_t addr, uint32_t len, const void *buf)
{
	uint32_t ret;
	uint8_t tx_buf[SPI_CMD_LEN + MAX_SPI_FRAMESIZE] = {0};
	uint8_t rx_buf[SPI_CMD_LEN + MAX_SPI_FRAMESIZE] = {0};

	if ((len != 1U) || (buf == NULL)) {
		printk(KERN_INFO "tpm spi write param err, len:0x%x\n", len);
		return TEE_ERR_BAD_PARAM;
	}

	tx_buf[0] = 0x0U | ((uint8_t)len - 1);
	tx_buf[ARR_IND_1] = (addr >> BIT_SHIFT_16) & 0xFFU;
	tx_buf[ARR_IND_2] = (addr >> BIT_SHIFT_8) & 0xFFU;
	tx_buf[ARR_IND_3] = (addr) & 0xFFU;

	(void)memcpy(&tx_buf[ARR_IND_4], buf, len);
	ioconfig_gpio(dev, bus);
	spi_cs_enable(dev);

	ret = tpm_spi_send_cmd(bus, dev, tx_buf, sizeof(tx_buf), rx_buf, sizeof(rx_buf));
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm spi write cmd err, 0x%x\n", ret);
		goto spi_release;
	}

	(void)memset(rx_buf, 0, sizeof(rx_buf));
	ret = spi_data_ctrl(bus, dev, &tx_buf[ARR_IND_4], 1, rx_buf, sizeof(rx_buf), 1);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm spi write data err, 0x%x\n", ret);
	}

spi_release:
	spi_cs_enable(dev);
	ioconfig_spi(dev, bus);

	return ret;
}

static uint32_t tpm_spi_read_byte(uintptr_t reg, uint8_t *ch)
{
	return tpm_spi_read(SPI_PORT_VAL, SPI_DEV_VAL, reg, 1, (void *)ch);
}

uint32_t read_tpm_driver(uintptr_t reg, uint8_t *ch)
{
	uint32_t ret = tpm_spi_read_byte(reg, ch);
	if (ret != TEE_OK) {
		printk(KERN_INFO "read tpm 0x%x err, 0x%x\n", reg, ret);
	}

	return ret;
}

static uint32_t tpm_spi_write_byte(uintptr_t reg, uint8_t *ch)
{
	return tpm_spi_write(SPI_PORT_VAL, SPI_DEV_VAL, reg, 1, (const void *)ch);
}

uint32_t write_tpm_driver(uintptr_t reg, uint8_t ch)
{
	uint32_t ret = tpm_spi_write_byte(reg, &ch);
	if (ret != TEE_OK) {
		printk(KERN_INFO "write tpm 0x%x err, 0x%x\n", reg, ret);
	}

	return ret;
}

