// SPDX-License-Identifier: GPL-2.0-only
/*
 * test_tpm.c - TPM test module
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License version 2 as
 * published by the Free Software Foundation.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 */

#include <linux/init.h>
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/kobject.h>
#include <linux/sysfs.h>
#include <linux/string.h>

#include "tpm_driver_cmd.h"
#include "tpm_driver_api.h"
#include "spi_drv.h"
#include "tpm_driver_common.h"

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Your Name");
MODULE_DESCRIPTION("Minimal sysfs example: echo 1 > /sys/kernel/test_tpm/test_tpm");

static struct kobject *test_kobj;


/*
 * sysfs 写回调：
 * 当用户执行 echo xxx > /sys/kernel/test_tpm/test_tpm 时被调用
 */
static ssize_t test_tpm_store(struct kobject *kobj,
							  struct kobj_attribute *attr,
							  const char *buf, size_t count)
{
	uint32_t ret;
	uint8_t recv_buf[TPM_RECV_CMD_LEN];
	uint32_t send_len, recv_len;
	uint8_t cmd[] = {
		0x80, 0x01,
		0x00, 0x00, 0x00, 0x16,
		0x00, 0x00, 0x01, 0x7A,
		0x00, 0x00, 0x00, 0x06,
		0x00, 0x00, 0x01, 0x00,
		0x00, 0x00, 0x00, 0x64
	};

	// 去除末尾换行符
	char val[32];
	size_t len = min(count, sizeof(val) - 1);
	memcpy(val, buf, len);
	val[len] = '\0';
	// 去掉可能存在的换行
	if (len > 0 && val[len-1] == '\n')
		val[len-1] = '\0';

	printk(KERN_INFO "test_tpm: received value = '%s'\n", val);

	ret = spi_driver_init();
	if (ret != TEE_OK) {
		printk(KERN_INFO "test_tpm spi init err, 0x%x\n", ret);
		return count;
	}

	ret = check_tpm_status();
	if (ret != TEE_OK) {
		printk(KERN_INFO "test_tpm check tpm err, 0x%x\n", ret);
		return count;
	}
	printk(KERN_INFO "test_tpm check tpm status ok\n");

	send_len = 22;
	recv_len = TPM_RECV_CMD_LEN;
	memset(recv_buf, 0, recv_len);
	ret = tpm_cmd_transmit(cmd, send_len, recv_buf, &recv_len);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm transmit error, ret=0x%x\n", ret);
		return -EIO;
	}

	print_hex_dump(KERN_INFO, "TPM recv: ", DUMP_PREFIX_OFFSET, 16, 1,
			recv_buf, recv_len, true);
	printk(KERN_INFO "test_tpm end\n");

	return count;
}

/* 定义属性 test_tpm，读写权限为 0220（仅 root 可写） */
static struct kobj_attribute test_tpm_attribute =
	__ATTR(test_tpm, 0220, NULL, test_tpm_store);

static struct attribute *attrs[] = {
	&test_tpm_attribute.attr,
	NULL,	/* 必须以 NULL 结尾 */
};

static struct attribute_group attr_group = {
	.attrs = attrs,
};

static int __init test_tpm_init(void)
{
	int ret;

	/* 在 /sys/kernel 下创建一个 kobject */
	test_kobj = kobject_create_and_add("test_tpm", kernel_kobj);
	if (!test_kobj)
		return -ENOMEM;

	/* 为这个 kobject 创建 sysfs 文件 */
	ret = sysfs_create_group(test_kobj, &attr_group);
	if (ret) {
		kobject_put(test_kobj);
		return ret;
	}

	printk(KERN_INFO "test_tpm: module loaded, interface at /sys/kernel/test_tpm/test_tpm\n");
	return 0;
}

static void __exit test_tpm_exit(void)
{
	spi_driver_exit();
	sysfs_remove_group(test_kobj, &attr_group);
	kobject_put(test_kobj);
	printk(KERN_INFO "test_tpm: module unloaded\n");
}

module_init(test_tpm_init);
module_exit(test_tpm_exit);
