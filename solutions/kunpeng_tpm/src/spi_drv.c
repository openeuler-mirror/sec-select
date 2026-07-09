#include <linux/types.h>

#include "tpm_driver_v2_interface.h"
#include "tpm_driver_gpio.h"
#include "tpm_driver_cmd.h"
#include "spi_drv.h"
#include "tpm_driver_spi_reg_v1.h"
#include "tpm_driver_spi_v1.h"
#include "tpm_driver_util.h"

#define REG_SIZE 0x1000

static struct spi_driver_info g_spi_info;
static struct mmap_info g_iomux_info;
static bool g_iomapped = false;

static struct spi_data *spi_get_drvdata(void)
{
	static struct spi_data s_spi_data = {0};

	return &s_spi_data;
}

void spi_driver_disable(uintptr_t vaddr)
{
	reg_write_32(vaddr + SPI_SSIENR_OFFSET, 0x0);
}

void spi_driver_enable(uintptr_t vaddr)
{
	reg_write_32(vaddr + SPI_SSIENR_OFFSET, 0x1);
}

uint32_t spi_fifo_rx_status(uintptr_t vaddr)
{
	uint32_t val;

	val = reg_read_32(vaddr + SPI_SR_OFFSET);
	if (0x0 == (val & SPI_RX_STATUS)) {
		return SPI_RX_FIFO_EMPTY_VAL;
	}

	return SPI_RX_FIFO_NOT_EMPTY_VAL;
}

uint32_t spi_data_ctrl(uint32_t port, uint32_t chip_select, uint8_t *cmd, uint32_t cmd_len, uint8_t *output,
			uint32_t output_max_len, uint32_t rx_len)
{
	uint32_t ret;
	struct spi_driver_info *spi_info = &g_spi_info;
	uint8_t rx_buf[SPI_TEMP_RX_BUFFER] = {0};
	struct spi_data *drvdata = spi_get_drvdata();

	if ((port > SPI_BUS_VAL) || (chip_select >= SPI_CHIP_SELECT_VAL)) {
		printk(KERN_INFO "spi data ctrl param err, 0x%x, 0x%x\n", port, chip_select);
		return TEE_ERR_BAD_PARAM;
	}

	if ((cmd_len > SPI_TEMP_RX_BUFFER) || (rx_len > SPI_TEMP_RX_BUFFER) ||
		((cmd_len + rx_len) > SPI_TEMP_RX_BUFFER)) {
		printk(KERN_INFO "spi data ctrl param len err, 0x%x, 0x%x\n", cmd_len, rx_len);
		return TEE_ERR_BAD_PARAM;
	}

	if ((drvdata == NULL) || (drvdata->ops == NULL) || (drvdata->ops->spi_write_and_read_8 == NULL)) {
		printk(KERN_INFO "spi data ctrl param drvdata err\n");
		return TEE_ERR_BAD_STATE;
	}

	ret = drvdata->ops->spi_write_and_read_8(spi_info, SPI_NON_EEPROM_RD, cmd, cmd_len, rx_buf, rx_len);
	if (ret != TEE_OK) {
		printk(KERN_INFO "spi data ctrl spi err, 0x%x\n", ret);
		return ret;
	}

	if ((cmd_len + rx_len) > output_max_len) {
		printk(KERN_INFO "spi data ctrl max len err\n");
		return TEE_ERR_BAD_STATE;
	}
	(void)memcpy((void *)output, (void *)rx_buf, cmd_len + rx_len);
	return 0;
}


static void spi_drv_init(void)
{
	struct spi_data *drvdata = spi_get_drvdata();
	if (drvdata->is_init == SPI_INIT_OK) {
		return;
	}

	init_spi_conf();
	drvdata->is_init = SPI_INIT_OK;
	return;
}

void register_spi_dev(struct spi_struct_ops *ops)
{
	struct spi_data *drvdata = spi_get_drvdata();

	drvdata->ops = ops;
}


uint32_t spi_driver_init(void)
{
	uint32_t port = SPI_PORT_VAL;
	uint32_t chip_select = SPI_DEV_VAL;
	struct spi_data *drvdata = spi_get_drvdata();
	struct mmap_info *gpio_info = gpio_get_info();

	if (gpio_info == NULL) {
		return TEE_ERR_BAD_STATE;
	}

	if (g_iomapped) {
		printk(KERN_INFO "spi driver already init\n");
		return TEE_OK;
	}

	spi_drv_init();

	g_iomux_info.vaddr[port] = ioremap(TPM_SPI_IO_MUX_BASE_ADDR, REG_SIZE);
	if (g_iomux_info.vaddr[port] == NULL) {
		printk(KERN_INFO "spi driver init g_iomux_info vaddr err\n");
		return TEE_ERR_BAD_STATE;
	}
	gpio_info->vaddr[port] = ioremap(TPM_GPIO_BASE_ADDR, REG_SIZE);
	if (gpio_info->vaddr[port] == NULL) {
		printk(KERN_INFO "spi driver init gpio_info vaddr err\n");
		iounmap((void *)g_iomux_info.vaddr[port]);
		g_iomux_info.vaddr[port] = NULL;
		return TEE_ERR_BAD_STATE;
	}

	g_spi_info.vaddr[port] = ioremap(TPM_SPI_BASE_ADDR, REG_SIZE);
	if (g_spi_info.vaddr[port] == NULL) {
		printk(KERN_INFO "spi driver init g_spi_info vaddr err\n");
		iounmap((void *)gpio_info->vaddr[port]);
		gpio_info->vaddr[port] = NULL;
		iounmap((void *)g_iomux_info.vaddr[port]);
		g_iomux_info.vaddr[port] = NULL;
		return TEE_ERR_BAD_STATE;
	}

	printk(KERN_INFO "spi driver init iomux:0x%lx, gpio:0x%lx, spi:0x%lx\n", g_iomux_info.vaddr[port], gpio_info->vaddr[port], g_spi_info.vaddr[port]);

	g_spi_info.port = port;
	g_spi_info.chip_select = chip_select;
	g_spi_info.attr[chip_select].baud_rate = SPI_BAUD_RATE;
	g_spi_info.attr[chip_select].frame_length = BYTE_7;
	g_spi_info.attr[chip_select].clock_edge = 0;
	g_spi_info.attr[chip_select].idle_clock_polarity = 0;

	reg_write_32(g_spi_info.vaddr[port] + SPI_SER_OFFSET, 0x01U);

	drvdata->ops->spi_conf(&g_spi_info, &g_iomux_info, gpio_info);

	g_iomapped = true;

	return TEE_OK;
}

void spi_driver_exit(void)
{
	uint32_t port = SPI_PORT_VAL;
	struct mmap_info *gpio_info = gpio_get_info();

	if (!g_iomapped) {
		return;
	}

	if (g_spi_info.vaddr[port] != NULL) {
		iounmap((void *)g_spi_info.vaddr[port]);
		g_spi_info.vaddr[port] = NULL;
	}

	if ((gpio_info != NULL) && (gpio_info->vaddr[port] != NULL)) {
		iounmap((void *)gpio_info->vaddr[port]);
		gpio_info->vaddr[port] = NULL;
	}

	if (g_iomux_info.vaddr[port] != NULL) {
		iounmap((void *)g_iomux_info.vaddr[port]);
		g_iomux_info.vaddr[port] = NULL;
	}

	g_iomapped = false;

}


void ioconfig_gpio(uint32_t cs, uint32_t bus)
{
	switch (cs) {
		case 0:
			reg_write_32(g_iomux_info.vaddr[bus] + IOMG_CSN_REG_ADDR, PAD_GPIO_CSN);
			break;
		default:
			printk(KERN_INFO "ioconfig gpio param err, 0x%x\n", cs);
			break;
	}
}

void ioconfig_spi(uint32_t cs, uint32_t bus)
{
	switch (cs) {
		case 0:
			reg_write_32(g_iomux_info.vaddr[bus] + IOMG_CSN_REG_ADDR, PAD_SPI_CSN);
			break;
		default:
			printk(KERN_INFO "ioconfig spi param err, 0x%x\n", cs);
			break;
	}
}

void spi_cs_enable(uint32_t cs)
{
	NO_USE_PARAM(cs);

	gpio_set_conf(SPI_CS0_GPIO_NUM, GPIO_CONF_DIR_OUT, GPIO_LEVEL_VAL_HIGH, g_spi_info.port);
}





