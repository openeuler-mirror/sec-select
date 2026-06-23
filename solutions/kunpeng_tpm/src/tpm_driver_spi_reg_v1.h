#ifndef TPM_DRIVER_SPI_REG_V1_H
#define TPM_DRIVER_SPI_REG_V1_H

#include <linux/types.h>

/* ==== Kunpeng 920B Platform Addresses ==== */
#define KUNPENG920B_SPI_BASE_ADDR	0x0004011A0000UL
#define KUNPENG920B_IOMUX_BASE_ADDR	0x000401100000UL
#define KUNPENG920B_GPIO_BASE_ADDR	0x000401120000UL

#define TPM_SPI_BASE_ADDR		KUNPENG920B_SPI_BASE_ADDR
#define TPM_SPI_IO_MUX_BASE_ADDR	KUNPENG920B_IOMUX_BASE_ADDR
#define TPM_GPIO_BASE_ADDR		KUNPENG920B_GPIO_BASE_ADDR

/* IOMUX register offsets for 920B */
#define IOMG_CLK_REG_ADDR	0x028U
#define IOMG_MOSI_REG_ADDR	0x02CU
#define IOMG_MISO_REG_ADDR	0x030U
#define IOMG_CSN_REG_ADDR	0x03CU

/* IOMUX PAD config values for 920B */
#define PAD_SPI_CLK	0x00U
#define PAD_SPI_MISO	0x00U
#define PAD_SPI_MOSI	0x00U
#define PAD_SPI_CSN	0x00U
#define PAD_GPIO_CSN	0x01U

/* GPIO number for SPI CS0 on 920B */
#define SPI_CS0_GPIO_NUM 7

/* SPI register offsets (DesignWare SPI) */
#define SPI_CTRLR0_OFFSET	0x00U
#define SPI_SSIENR_OFFSET	0x08U
#define SPI_SER_OFFSET		0x10U
#define SPI_BAUDR_OFFSET	0x14U
#define SPI_TXFLR_OFFSET	0x20U
#define SPI_RXFLR_OFFSET	0x24U
#define SPI_SR_OFFSET		0x28U
#define SPI_IMR_OFFSET		0x2cU
#define RX_SAMPLE_DLY		0xF0U
#define SPI_DR0_OFFSET		0x60U
#define SPI_RX_STATUS		0x8U

typedef union {
	struct {
		uint32_t reserved_dfs : 4;
		uint32_t frf : 2;
		uint32_t scph : 1;
		uint32_t scpol : 1;
		uint32_t tmod : 2;
		uint32_t reserved_0 : 1;
		uint32_t srl : 1;
		uint32_t cfs : 4;
		uint32_t dfs : 5;
		uint32_t reserved_1 : 11;
	} bits;
	uint32_t val_32;
} spi_ctrl_u;

#endif
