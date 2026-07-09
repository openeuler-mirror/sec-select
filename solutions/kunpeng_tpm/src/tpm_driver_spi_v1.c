#include <linux/types.h>
#include "tpm_driver_util.h"

#include "tpm_driver_v2_interface.h"
#include "tpm_driver_gpio.h"
#include "tpm_driver_cmd.h"
#include "spi_drv.h"
#include "tpm_driver_spi_reg_v1.h"
#include "tpm_driver_spi_v1.h"

static uint32_t check_spi_tx_fifo_level(uintptr_t vaddr, uint32_t timeout)
{
	uint32_t value = 0;
	uint32_t cnt;

	for (cnt = 0; cnt < timeout; cnt++) {
		value = reg_read_32(vaddr + SPI_TXFLR_OFFSET);
		if (value == 0) {
			return TEE_OK;
		}
	}

	printk(KERN_INFO "check spi tx read time out, 0x%x\n", value);
	return TEE_ERR_BAD_STATE;
}

static uint32_t get_spi_rx_fifo_level(uintptr_t vaddr, uint32_t timeout)
{
	uint32_t value = 0;
	uint32_t cnt;

	for (cnt = 0; cnt < timeout; cnt++) {
		value = reg_read_32(vaddr + SPI_RXFLR_OFFSET);
		if (value != 0) {
			return value;
		}
	}

	printk(KERN_INFO "get spi rx read time out\n");
	return 0;
}

static uint32_t check_spi_transfer_status(uintptr_t vaddr)
{
	uint32_t level, status;
	uint32_t wait_times;
	uint32_t ret;

	ret = check_spi_tx_fifo_level(vaddr, SPI_TIMEOUT);
	if (ret != TEE_OK) {
		return ret;
	}

	level = get_spi_rx_fifo_level(vaddr, SPI_TIMEOUT);
	if (level == 0) {
		return TEE_ERR_BAD_STATE;
	}

	for (wait_times = 0; wait_times < SPI_WAIT_TIMES; wait_times++) {
		status = spi_fifo_rx_status(vaddr);
		if (status == SPI_RX_FIFO_NOT_EMPTY_VAL) {
			return TEE_OK;
		}
	}

	printk(KERN_INFO "check spi status err, 0x%x\n", level);
	return TEE_ERR_BAD_STATE;
}

static uint32_t read_spi_mode_8(uintptr_t vaddr, uint8_t *recv_buf, __attribute__((unused)) uint32_t recv_len)
{
	uint32_t trans_val;
	uint32_t ret;

	reg_write_32(vaddr + SPI_DR0_OFFSET, SPI_START_RX_VAL);

	ret = check_spi_transfer_status(vaddr);
	if (ret != TEE_OK) {
		printk(KERN_INFO "read spi check err 0x%x\n", ret);
		return ret;
	}

	trans_val = reg_read_32(vaddr + SPI_DR0_OFFSET);
	recv_buf[0] = (uint8_t)(trans_val & 0xffU);

	return TEE_OK;
}

static uint32_t write_spi_mode_8(uintptr_t vaddr, uint8_t *send_buf, uint32_t send_len, uint8_t *recv_buf,
			__attribute__((unused)) uint32_t recv_len)
{
	uint32_t i;
	uint32_t trans_val;
	uint32_t ret;

	for (i = 0; i < send_len; i++) {
		trans_val = (uint32_t)(send_buf[i] & 0xffU);
		reg_write_32(vaddr + SPI_DR0_OFFSET, trans_val);

		ret = check_spi_transfer_status(vaddr);
		if (ret != TEE_OK) {
			printk(KERN_INFO "write spi transfer err, 0x%x, 0x%x, 0x%x\n", ret, i, trans_val);
			return ret;
		}

		trans_val = reg_read_32(vaddr + SPI_DR0_OFFSET);
		recv_buf[i] = (uint8_t)(trans_val & 0xffU);
	}

	return TEE_OK;
}

static uint32_t spi_write_and_read_8(struct spi_driver_info *spi_info, uint32_t transmode, uint8_t *send_buf,
			uint32_t send_len, uint8_t *recv_buf, uint32_t recv_len)
{
	spi_ctrl_u tmp_val;
	uint32_t port = spi_info->port;
	uintptr_t vaddr = spi_info->vaddr[port];

	if (vaddr == 0) {
		printk(KERN_INFO "spi write and read 8 vaddr err\n");
		return TEE_ERR_BAD_STATE;
	}

	spi_driver_disable(vaddr);
	tmp_val.val_32 = reg_read_32(vaddr + SPI_CTRLR0_OFFSET);
	if (transmode == SPI_NON_EEPROM_RD) {
		tmp_val.bits.tmod = SPI_TMOD_DUPLEX;
	} else {
		tmp_val.bits.tmod = SPI_TMOD_EEPROM_RD;
	}
	reg_write_32(vaddr + SPI_CTRLR0_OFFSET, tmp_val.val_32);
	spi_driver_enable(vaddr);

	gpio_set_conf(SPI_CS0_GPIO_NUM, GPIO_CONF_DIR_OUT, GPIO_LEVEL_VAL_LOW, port);
	gpio_set_level(SPI_CS0_GPIO_NUM, GPIO_LEVEL_VAL_LOW, port);

	if (send_len == 0) {
		return read_spi_mode_8(vaddr, recv_buf, recv_len);
	}

	return write_spi_mode_8(vaddr, send_buf, send_len, recv_buf, recv_len);
}

static void spi_conf(struct spi_driver_info *spi_info, struct mmap_info *iomux_info,
		__attribute__((unused)) struct mmap_info *gpio_info)
{
	spi_ctrl_u tmp_val;
	uint32_t freq;
	uint32_t spi_freq;
	uint32_t port = spi_info->port;
	uint32_t chip_select = spi_info->chip_select;

	spi_driver_disable(spi_info->vaddr[port]);
	spi_freq = SPI_FREQ;

	freq = spi_freq / (spi_info->attr[chip_select].baud_rate);

	if ((freq & 0x1) != 0) {
		freq = freq + 1;
	}

	reg_write_32(spi_info->vaddr[port] + SPI_BAUDR_OFFSET, freq);
	reg_write_32(spi_info->vaddr[port] + SPI_IMR_OFFSET, 0x00);

	tmp_val.val_32 = reg_read_32(spi_info->vaddr[port] + SPI_CTRLR0_OFFSET);

	tmp_val.bits.srl = 0x0;
	tmp_val.bits.frf = 0x0;
	tmp_val.bits.dfs = spi_info->attr[chip_select].frame_length;
	tmp_val.bits.scpol = spi_info->attr[chip_select].idle_clock_polarity;
	tmp_val.bits.scph = spi_info->attr[chip_select].clock_edge;

	reg_write_32(spi_info->vaddr[port] + SPI_CTRLR0_OFFSET, tmp_val.val_32);
	reg_write_32(spi_info->vaddr[port] + RX_SAMPLE_DLY, 0x02U);

	switch (port) {
		case 1:
			reg_write_32(iomux_info->vaddr[port] + IOMG_CLK_REG_ADDR, PAD_SPI_CLK);
			reg_write_32(iomux_info->vaddr[port] + IOMG_MOSI_REG_ADDR, PAD_SPI_MOSI);
			reg_write_32(iomux_info->vaddr[port] + IOMG_MISO_REG_ADDR, PAD_SPI_MISO);
			reg_write_32(iomux_info->vaddr[port] + IOMG_CSN_REG_ADDR, PAD_GPIO_CSN);
			break;
		default:
			break;
	}

	spi_driver_enable(spi_info->vaddr[port]);
}

static struct spi_struct_ops g_spi = {
	.spi_write_and_read_16 = NULL,
	.spi_write_and_read_8 = spi_write_and_read_8,
	.spi_conf = spi_conf,
};

void init_spi_conf(void)
{
	register_spi_dev(&g_spi);
}

