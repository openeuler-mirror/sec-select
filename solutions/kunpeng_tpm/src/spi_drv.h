#ifndef SPI_DRV_H
#define SPI_DRV_H

#include "tpm_driver_gpio.h"

#define SPI_PORT_VAL 1U
#define SPI_DEV_VAL 0U

#define SPI_CHIP_SELECT_VAL 1U
#define SPI_BUS_VAL 1U
#define SPI_START_RX_VAL 0x5aU
#define SPI_RX_FIFO_EMPTY_VAL 0U
#define SPI_RX_FIFO_NOT_EMPTY_VAL 1U

#define SPI_TEMP_RX_BUFFER 32U
#define SPI_WAIT_TIMES 5000U
#define SPI_TMOD_DUPLEX 0U
#define SPI_TMOD_EEPROM_RD 3U
#define SPI_TIMEOUT 0x64U
#define SPI_FREQ 250000000U
#define SPI_BAUD_RATE 18000000U
#define SPI_INIT_OK 0x1U

typedef enum {
	SPI_NON_EEPROM_RD = 0,
	SPI_EEPROM_RD
} SPI_TMODE;

typedef struct {
	uint32_t baud_rate;
	uint32_t frame_length;
	uint32_t clock_edge;
	uint32_t idle_clock_polarity;
} SPI_ATTR;

struct spi_driver_info {
	uint32_t port;
	uint32_t chip_select;
	uintptr_t paddr[2];
	uintptr_t vaddr[2];
	SPI_ATTR attr[2];
};

struct spi_struct_ops {
	uint32_t (*spi_write_and_read_16)(struct spi_driver_info *spi_info, uint32_t ulRdMode, uint8_t *pucTxBuf,
			uint32_t ulTxLen, uint8_t *pucRxBuf, uint32_t ulRxLen);
	uint32_t (*spi_write_and_read_8)(struct spi_driver_info *spi_info, uint32_t transmode, uint8_t *send_buf,
			uint32_t send_len, uint8_t *recv_buf, uint32_t recv_len);
	void (*spi_conf)(struct spi_driver_info *spi_info, struct mmap_info *iomux_info, struct mmap_info *gpio_info);
};

struct spi_data {
	struct spi_struct_ops *ops;
	bool is_init;
};

uint32_t spi_driver_init(void);
void spi_driver_exit(void);
uint32_t spi_data_ctrl(uint32_t port, uint32_t chip_select, uint8_t *cmd, uint32_t cmd_len, uint8_t *output,
			uint32_t output_max_len, uint32_t rx_len);

void spi_driver_disable(uintptr_t vaddr);
void spi_driver_enable(uintptr_t vaddr);
uint32_t spi_fifo_rx_status(uintptr_t vaddr);
void register_spi_dev(struct spi_struct_ops *ops);
void ioconfig_gpio(uint32_t cs, uint32_t bus);
void ioconfig_spi(uint32_t cs, uint32_t bus);
void spi_cs_enable(uint32_t cs);

#endif
