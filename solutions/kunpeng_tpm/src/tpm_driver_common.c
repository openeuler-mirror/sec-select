#include "tpm_driver_common.h"

static unsigned long get_time_ns(void)
{
	unsigned long val, freq;
	asm volatile("mrs %0, cntvct_el0" : "=r" (val));
	asm volatile("mrs %0, cntfrq_el0" : "=r" (freq));

	if (freq == 0)
		return 0;

	return 1000000000 / freq * val;
}

void tpm_delay_with_us(unsigned long usec)
{
	unsigned long  start, now, current_ns;

	start = get_time_ns();
	while (1) {
		current_ns = get_time_ns();
		if (current_ns - start >= usec * 1000)
			break;
	}
}
