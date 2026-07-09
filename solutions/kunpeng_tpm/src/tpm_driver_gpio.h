#ifndef TPM_DRIVER_GPIO_H
#define TPM_DRIVER_GPIO_H

#include <linux/types.h>
#include <linux/io.h>
#include "tpm_driver_util.h"

#define GPIO_CONF_NUM_MAX 256U
#define GPIO_CONF_NUM_PER_GROUP 32U
#define GPIO_CONF_SWPORT_DR 0x00U
#define GPIO_CONF_SWPORT_DDR 0x04U
#define GPIO_CONF_DEBOUNCE 0x48U
#define GPIO_CONF_DIR_IN 0U
#define GPIO_CONF_DIR_OUT 1U
#define GPIO_LEVEL_VAL_LOW 0U
#define GPIO_LEVEL_VAL_HIGH 1U


static inline void reg_data_sync(void)
{
	asm volatile("dsb sy");
}

static inline void reg_write_u32(uint32_t val, volatile void *addr)
{
	asm volatile("str %w0, [%1]" : : "rZ" (val), "r" (addr));
}

static inline uint32_t reg_read_u32(const volatile void *addr)
{
	uint32_t val;
	asm volatile("ldr %w0, [%1]" : "=r" (val) : "r" (addr));
	return val;
}

#define reg_read_32(addr) reg_read_u32((volatile unsigned *)(uintptr_t)(addr))

static inline void reg_write_32(unsigned long addr, unsigned val)
{
	reg_data_sync();
	reg_write_u32(val, (volatile unsigned *)(uintptr_t)(addr));
	reg_data_sync();
}

struct mmap_info {
	uintptr_t paddr[2];
	uintptr_t vaddr[2];
};

static inline uint32_t reg_get_mask(uint32_t bit_start, uint32_t bit_end)
{
	uint32_t mask = 0;
	uint32_t bit_s = bit_start;
	uint32_t bit_e = bit_end;

	if (bit_e == 31U) {
		mask = 0x80000000U;
		bit_e--;
	}

	mask |= (uint32_t)((0x1U << (bit_e + 1)) - (0x1U << bit_s));

	return mask;
}

static inline void reg_clear_bit_32(uintptr_t addr, uint32_t bit_start, uint32_t bit_end)
{
	if ((bit_start > 31U) || (bit_end > 31U) || (bit_end < bit_start)) {
		return;
	}

	uint32_t mask = reg_get_mask(bit_start, bit_end);
	uint32_t val = reg_read_32(addr);
	val &= (~mask);

	reg_write_32(addr, val);
}

static inline void reg_set_bit_32(uintptr_t addr, uint32_t bit_start, uint32_t bit_end)
{
	if ((bit_start > 31U) || (bit_end > 31U) || (bit_end < bit_start)) {
		return;
	}

	uint32_t mask = reg_get_mask(bit_start, bit_end);
	uint32_t val = reg_read_32(addr);
	val |= mask;
	reg_write_32(addr, val);
}

struct mmap_info *gpio_get_info(void);
void gpio_set_level(uint32_t pin, uint32_t level, uint32_t port);
void gpio_set_conf(uint32_t pin, uint32_t dir, uint32_t level, uint32_t port);

#endif
