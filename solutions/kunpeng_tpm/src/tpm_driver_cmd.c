#include "tpm_driver_util.h"
#include "tpm_driver_v2_interface.h"
#include "spi_drv.h"
#include "tpm_driver_ops.h"
#include "tpm_driver_cmd.h"

static uint32_t write_tpm_bytes(uint32_t addr, uint8_t *buf, uint32_t len, bool is_addr_inc)
{
	uint32_t off, ret;

	for (uint32_t i = 0; i < len; i++) {
		off = addr + (is_addr_inc ? i : 0);
		ret = write_tpm_driver(off, buf[i]);
		if (ret != TEE_OK) {
			printk(KERN_INFO "write tpm bytes err, ret:%d\n", ret);
			return ret;
		}
	}

	return TEE_OK;
}

static uint32_t read_tpm_bytes(uintptr_t addr, uint8_t *buf, uint32_t len, bool is_addr_inc)
{
        uintptr_t off;
        uint32_t ret;

        for (uint32_t i = 0; i < len; i++) {
                off = addr + (is_addr_inc ? i : 0);
                ret = read_tpm_driver(off, &buf[i]);
                if (ret != TEE_OK) {
                        printk(KERN_INFO "read tpm bytes err, ret:%d\n", ret);
                        return ret;
                }
        }

        return TEE_OK;
}

static bool check_tpm_cmd_status(uint8_t ch)
{
	if (((ch >> BIT_SHIFT_6) & 0x1U) == 1U) {
		return true;
	}
	return false;
}

static uint32_t wait_for_tpm_status(uintptr_t reg, uint8_t mask, uint8_t expect, uint32_t timeout)
{
	uint32_t ret;
	uint32_t cnt = 0;
	uint8_t status;

	do {
		status = 0;
		ret = read_tpm_driver(reg, &status);
		if ((ret == TEE_OK) && ((status & mask) == expect)) {
			return TEE_OK;
		}
		cnt++;
		tpm_delay_with_us(TPM_DELAY_TIME);
	} while (cnt < timeout);

	printk(KERN_INFO "tpm stat 0x%lx fail, 0x%x|0x%x|0x%x|0x%x|0x%x|0x%x.\n",
			reg, ret, cnt, status, mask, expect, timeout);
	return TEE_ERR_BUSY;
}

static uint32_t check_tpm_send_cmd_status(uint32_t locality)
{
	uint8_t status;

	for (uint32_t i = 0; i < TPM_WAIT_LOOP; i++) {
		status = 0;
		(void)write_tpm_driver(TPM_SYS_1(locality), TPM_STS_COMMAND_READY);
		(void)read_tpm_driver(TPM_SYS_1(locality), &status);

		if (check_tpm_cmd_status(status)) {
			return TEE_OK;
		}
		tpm_delay_with_us(TPM_CMD_READY_WAIT_TIME);
	}

	printk(KERN_INFO "tpm loc 0x%x send cmd ready failed, 0x%x.\n", locality, status);
	return TEE_ERR_BAD_STATE;
}

static void get_tpm_chip_id(void)
{
	uint32_t ret;
	uint32_t tpm_id = 0;
	static uint8_t s_chip_id_flag = 0;

	if (s_chip_id_flag == 0) {
		ret = read_tpm_bytes(TPM_ID_ADDR_0, (uint8_t *)(uintptr_t)&tpm_id, BYTE_4, 1);
		if (ret != TEE_OK) {
			printk(KERN_INFO "get tpm id failed, 0x%x.\n", ret);
		} else {
			printk(KERN_INFO "get tpm id, 0x%x.\n", tpm_id);
		}

		s_chip_id_flag = 1;
	}
}

static void release_tpm_locality(uint32_t locality)
{
	uint32_t ret;

	(void)write_tpm_driver(TPM_ACCESS(locality), TPM_ACCESS_ACTIVE_LOCALITY);
	ret = wait_for_tpm_status(TPM_ACCESS(locality), TPM_ACCESS_ACTIVED_MASK, TPM_ACCESS_VALID, TPM2_TIMEOUT_A);
	if (ret != TEE_OK) {
		printk(KERN_INFO "release tpm locality 0x%x err, 0x%x.\n", locality, ret);
		return;
	}
}

static void release_all_tpm_locality(void)
{
	uint8_t access;

	for (uint32_t i = 0; i < TPM_MAX_LOCALITY; i++) {
		access = 0;
		(void)read_tpm_driver(TPM_ACCESS(i), &access);
		if (IS_TPM_ACCESS_ACTIVED(access)) {
			release_tpm_locality(i);
		}
	}
}


static uint32_t request_tpm_locality(uint32_t locality)
{
	uint32_t ret;

	(void)write_tpm_driver(TPM_ACCESS(locality), TPM_ACCESS_REQUEST_USE);

	ret = wait_for_tpm_status(TPM_ACCESS(locality), TPM_ACCESS_ACTIVED_MASK, TPM_ACCESS_ACTIVED_MASK, TPM2_TIMEOUT_A);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm access 0x%x state err, 0x%x.\n", locality, ret);
		return TEE_ERR_BAD_STATE;
	}

	return TEE_OK;
}


static uint32_t request_tpm_locality_start(uint32_t locality)
{
	uint32_t i = 0;
	uint32_t ret;

	release_all_tpm_locality();
	while (i < TPM_ACCESS_TRY_TIME) {
		ret = request_tpm_locality(locality);
		if (ret == TEE_OK) {
			break;
		}
		release_all_tpm_locality();
		tpm_delay_with_us(TPM_DELAY_TIME);
		i++;
	}

	if (i >= TPM_ACCESS_TRY_TIME) {
		printk(KERN_INFO "try access serveral times failed, 0x%x.\n", locality);
		return TEE_ERR_BAD_STATE;
	}

	return TEE_OK;
}

static uint32_t get_tpm_burstcount(uint32_t locality, uint32_t *result)
{
	uint32_t ret;
	uint32_t burstcnt;
	uint32_t value;
	uint32_t cnt = 0;

	do {
		value = 0;
		ret = read_tpm_bytes(TPM_SYS_1(locality), (uint8_t *)&value, BYTE_4, 1);
		if (ret != TEE_OK) {
			printk(KERN_INFO "get tpm burstcnt failed, 0x%x\n", ret);
			return ret;	
		}

		burstcnt = ((value >> BIT_SHIFT_8) & 0xFFFFU);
		if (burstcnt != 0) {
			*result = burstcnt;
			return TEE_OK;
		}
		cnt++;
		tpm_delay_with_us(TPM_DELAY_TIME);
	} while (cnt < TPM2_TIMEOUT_A);

	printk(KERN_INFO "get tpm burstcnt err 0x%x value 0x%x.\n", burstcnt, value);
	return TEE_ERR_BAD_STATE;
}


static uint32_t send_tpm_cmd_data(uint32_t locality, uint8_t *send_buf, uint32_t len, uint32_t *send_cnt)
{
	uint32_t ret;
	uint32_t burstcnt = 0;
	uint32_t cnt = 0;
	uint32_t cur_len;

	if ((send_buf == NULL) || (len < TPM_CMD_HEADER_SIZE)) {
		printk(KERN_INFO "param is invalid 0x%x.\n", len);
		return TEE_ERR_BAD_PARAM;
	}

	*send_cnt = ((uint32_t)send_buf[ARR_IND_2] << BIT_SHIFT_24);
	*send_cnt |= ((uint32_t)send_buf[ARR_IND_3] << BIT_SHIFT_16);
	*send_cnt |= ((uint32_t)send_buf[ARR_IND_4] << BIT_SHIFT_8);
	*send_cnt |= ((uint32_t)send_buf[ARR_IND_5]);

	if ((*send_cnt > len) || (*send_cnt < TPM_CMD_HEADER_SIZE)) {
		printk(KERN_INFO "tpm send len invalid 0x%x|0x%x.\n", *send_cnt, len);
		return TEE_ERR_BAD_PARAM;
	}

	while (cnt < (*send_cnt - 1)) {
		ret = get_tpm_burstcount(locality, &burstcnt);
		if (ret != TEE_OK) {
			printk(KERN_INFO "tpm unable to read burstcnt, 0x%x|0x%x.\n", ret, burstcnt);
			return TEE_ERR_BAD_PARAM;
		}

		cur_len = MIN(burstcnt, MAX_SPI_FRAMESIZE);
		burstcnt = MIN((*send_cnt - 1 - cnt), cur_len);
		ret = write_tpm_bytes(TPM_DATA_FIFO_1(locality), send_buf + cnt, burstcnt, 0);
		if (ret != TEE_OK) {
			return ret;
		}

		cnt += burstcnt;

		ret = wait_for_tpm_status(TPM_SYS_1(locality), (TPM_STS_VALID | TPM_STS_DATA_EXPECT),
				(TPM_STS_VALID | TPM_STS_DATA_EXPECT), TPM2_TIMEOUT_A);
		if (ret != TEE_OK) {
			printk(KERN_INFO "tpm expect state 0x%x err, 0x%x.\n", locality, ret);
			return TEE_ERR_BAD_STATE;	
		}
	}

	(void)write_tpm_bytes(TPM_DATA_FIFO_1(locality), &(send_buf[cnt]), 1, 0);

	ret = wait_for_tpm_status(TPM_SYS_1(locality), (TPM_STS_VALID | TPM_STS_DATA_EXPECT), TPM_STS_VALID, TPM2_TIMEOUT_A);
	if (ret != TEE_OK) {
		printk(KERN_INFO "after while tpm expect state 0x%x err, 0x%x.\n", locality, ret);
		return TEE_ERR_BAD_STATE;
	}

	(void)write_tpm_driver(TPM_SYS_1(locality), TPM_STS_TPMGO);

	return TEE_OK;
}

static uint32_t recv_tpm_data(uint32_t locality, uint8_t *rx_buf, uint32_t cnt, uint32_t *recv_size)
{
	uint32_t ret;
	uint32_t size = 0;
	uint32_t burstcnt = 0;
	uint32_t cur_len;

	while (size < cnt) {
		ret = wait_for_tpm_status(TPM_SYS_1(locality), (TPM_STS_VALID | TPM_STS_DATA_AVAIL),
				(TPM_STS_VALID | TPM_STS_DATA_AVAIL), TPM2_TIMEOUT_A);
		if (ret != TEE_OK) {
			break;
		}

		ret = get_tpm_burstcount(locality, &burstcnt);
		if (ret != TEE_OK) {
			printk(KERN_INFO "recv tpm read brustcnt err 0x%x, 0x%x\n", ret, burstcnt);
			return ret;
		}

		cur_len = MIN(MAX_SPI_FRAMESIZE, burstcnt);
		burstcnt = MIN(cur_len, (cnt - size));
		ret = read_tpm_bytes(TPM_DATA_FIFO_1(locality), rx_buf + size, burstcnt, 0);
		if (ret != TEE_OK) {
			printk(KERN_INFO "recv tpm read data fifo failed, 0x%x\n", ret);
			return ret;
		}

		size += burstcnt;
	}

	*recv_size = size;

	return TEE_OK;
}


static uint32_t recv_tpm_cmd_data(uint32_t locality, uint8_t *send_buf, uint32_t send_cnt, uint8_t *resp_buf, uint32_t *resp_len)
{
	NO_USE_PARAM(send_cnt);
	uint32_t j;
	uint32_t ret;
	uint32_t recv_len;
	uint32_t size = 0;
	uint32_t rc = 0;

	if (*resp_len < TPM_CMD_HEADER_SIZE) {
		printk(KERN_INFO "tpm buf len invalid,  0x%x\n", *resp_len);
		return TEE_ERR_BAD_PARAM;
	}

	ret = recv_tpm_data(locality, resp_buf, TPM_CMD_HEADER_SIZE, &size);
	if ((ret != TEE_OK) || (size < TPM_CMD_HEADER_SIZE)) {
		printk(KERN_INFO "tpm read frame header err, 0x%x, 0x%x\n", ret, size);
		return TEE_ERR_OUT_OF_MEMORY;
	}

	recv_len = ((uint32_t)resp_buf[ARR_IND_2] << BIT_SHIFT_24);
	recv_len |= ((uint32_t)resp_buf[ARR_IND_3] << BIT_SHIFT_16);
	recv_len |= ((uint32_t)resp_buf[ARR_IND_4] << BIT_SHIFT_8);
	recv_len |= ((uint32_t)resp_buf[ARR_IND_5]);

	if ((recv_len < TPM_CMD_HEADER_SIZE) || (recv_len > TPM_RECV_CMD_LEN)) {
		printk(KERN_INFO "tpm recv len err, 0x%x, 0x%x\n", recv_len, size);
		for (j = 0; j < TPM_CMD_HEADER_SIZE; j++) {
			printk(KERN_INFO "send_buf: 0x%x.\n", send_buf[j]);
		}
		printk(KERN_INFO "tpm recv frame data is\n");
		for (j = 0; j < TPM_CMD_HEADER_SIZE; j++) {
			printk(KERN_INFO "resp_buf:0x%x.\n", resp_buf[j]);
		}
		return TEE_ERR_BAD_STATE;
	}

	ret = recv_tpm_data(locality, resp_buf + TPM_CMD_HEADER_SIZE, recv_len - TPM_CMD_HEADER_SIZE, &rc);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm read data err, 0x%x,0x%x\n", ret, rc);
		return TEE_ERR_BAD_STATE;
	}
	
	size += rc;
	if (size < recv_len) {
		printk(KERN_INFO "tpm read data err at 0x%x, 0x%x, 0x%x\n", recv_len, size, rc);
		return TEE_ERR_OUT_OF_MEMORY;
	}
	
	*resp_len = recv_len;

	return TEE_OK;
}

static uint32_t transfer_tpm_cmd_data(uint32_t locality, uint8_t *send_buf, uint32_t len, uint8_t *resp_buf, uint32_t *resp_len)
{
	uint32_t send_cnt = 0;
	uint32_t ret;

	get_tpm_chip_id();

	ret = request_tpm_locality_start(locality);
	if (ret != TEE_OK) {
		ret = TEE_ERR_BUSY;
		goto out;
	}

	ret = check_tpm_send_cmd_status(locality);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm send cmd status failed, 0x%x\n", ret);
		ret = TEE_ERR_BUSY;
		goto out;
	}

	ret = send_tpm_cmd_data(locality, send_buf, len, &send_cnt);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm send cmd data failed, 0x%x\n", send_cnt);
		ret = TEE_ERR_BUSY;
		goto out;
	}

	ret = wait_for_tpm_status(TPM_SYS_1(locality), (TPM_STS_VALID | TPM_STS_DATA_AVAIL),
			(TPM_STS_VALID | TPM_STS_DATA_AVAIL), TPM2_DURATION_LONG_LONG);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm data state failed, 0x%x, 0x%x\n", locality, ret);
		ret = TEE_ERR_BAD_STATE;
		goto out;
	}

	ret = recv_tpm_cmd_data(locality, send_buf, send_cnt, resp_buf, resp_len);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm recv data failed, 0x%x\n", ret);
		ret = TEE_ERR_BUSY;
		goto out;
	}

	ret = wait_for_tpm_status(TPM_SYS_1(locality), (TPM_STS_VALID | TPM_STS_DATA_AVAIL), TPM_STS_VALID, TPM2_TIMEOUT_A);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm recv data state failed, 0x%x, 0x%x\n", locality, ret);
		ret = TEE_ERR_BAD_STATE;
	}
out:
	(void)check_tpm_send_cmd_status(locality);
	release_tpm_locality(locality);

	return ret;
}


uint32_t tpm_cmd_transmit(uint8_t *send_data, uint32_t send_len, uint8_t *recv_data, uint32_t *recv_len)
{
	uint32_t ret;
	uint32_t locality = 0;

	if ((send_data == NULL) || (recv_data == NULL) || (recv_len == NULL)) {
		printk(KERN_INFO "tpm transmit input err\n");
		return TEE_ERR_BAD_PARAM;
	}

	if ((*recv_len != TPM_RECV_CMD_LEN) || (send_len < TPM_CMD_HEADER_SIZE) || (send_len > TPM_SEND_CMD_LEN)) {
		printk(KERN_INFO "tpm transmit input len err, 0x%x|0x%x|0x%x|0x%x|0x%x\n", *recv_len, send_len,
				TPM_RECV_CMD_LEN, TPM_CMD_HEADER_SIZE, TPM_SEND_CMD_LEN);
		return TEE_ERR_BAD_PARAM;
	}

	ret = spi_driver_init();
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm transmit spi init err,0x%x\n", ret);
		return ret;
	}

	ret = transfer_tpm_cmd_data(locality, send_data, send_len, recv_data, recv_len);
	if (ret != TEE_OK) {
		printk(KERN_INFO "tpm transmit cmd data err,0x%x\n", ret);
		return ret;
	}

	return TEE_OK;
}



