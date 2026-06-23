#include <linux/types.h>
#include "spi_drv.h"
#include "tpm_driver_gpio.h"

static struct mmap_info g_gpio_info;

struct mmap_info *gpio_get_info(void)
{
	return &g_gpio_info;
}

void gpio_set_level(uint32_t pin, uint32_t level, uint32_t port)
{
	uintptr_t base;
	uint32_t index;

	if (port > 1) {
		printk(KERN_INFO "gpio set level input port err, %d\n", port);
		return;
	}

	if (pin >= GPIO_CONF_NUM_MAX) {
		printk(KERN_INFO "gpio set level input pin err, 0x%x\n", pin);
		return;
	}

	base = g_gpio_info.vaddr[port];
	index = pin % GPIO_CONF_NUM_PER_GROUP;
	if (level == GPIO_LEVEL_VAL_LOW) {
		reg_clear_bit_32(base + GPIO_CONF_SWPORT_DR, index, index);
	} else {
		reg_set_bit_32(base + GPIO_CONF_SWPORT_DR, index, index);
	}
}

void gpio_set_conf(uint32_t pin, uint32_t dir, uint32_t level, uint32_t port)
{
	uintptr_t base;
	uint32_t index;

	if (port > 1) {
		printk(KERN_INFO "gpio set conf input port err, %d\n", port);
		return;
	}

	if (pin >= GPIO_CONF_NUM_MAX) {
		printk(KERN_INFO "gpio set conf input pin err, 0x%x\n", pin);
		return;
	}

	base = g_gpio_info.vaddr[port];
	index = pin % GPIO_CONF_NUM_PER_GROUP;
	if (dir == GPIO_CONF_DIR_IN) {
		reg_set_bit_32(base + GPIO_CONF_DEBOUNCE, index, index);
		reg_clear_bit_32(base + GPIO_CONF_SWPORT_DDR, index, index);
	} else {
		gpio_set_level(pin, level, port);
		reg_set_bit_32(base + GPIO_CONF_SWPORT_DDR, index, index);
	}
}
