obj-m := test_tpm_driver.o

test_tpm_driver-objs := test_tpm.o tpm_driver_cmd.o tpm_driver_ops.o spi_drv.o \
			tpm_driver_spi_v1.o tpm_driver_gpio.o tpm_driver_common.o tpm_driver_api.o

KDIR := /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
