#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <sys/ioctl.h>

/*
 * 内核 rem extend 适配 -- 通过 tsm_ops->measurement_extend 调用
 *
 * 已将 tsm_ops 的 measurement_extend 函数指针注册为
 * arm_cca_measurement_extend, 见 include/linux/tsm.h:
 *   int (*measurement_extend)(struct tsm_measurement *measurement, void *data);
 *
 * 调用接口结构体(include/linux/tsm.h):
 *   #define TSM_MEASUREMENT_MAX_SIZE 64
 *   struct tsm_measurement {
 *       unsigned int index;
 *       size_t value_len;
 *       u8 value[TSM_MEASUREMENT_MAX_SIZE];
 *   };
 *
 * 内部调用链(含完整函数签名):
 *   1. arm_cca_measurement_extend(struct tsm_measurement *measurement, void *data)
 *      - measurement->index    : REM 索引
 *      - measurement->value    : 测量值字节
 *      - measurement->value_len: 测量值长度
 *      - data                  : provider 数据(未使用)
 *
 *   2.   -> rsi_measurement_extend(unsigned long index,
 *                                  const u8 *measurement,
 *                                  unsigned long size)
 *        - index     : measurement->index
 *        - measurement: measurement->value
 *        - size      : measurement->value_len
 *
 *   3.     -> arm_smccc_1_2_smc(&regs, &regs)
 *          - regs.a0 = SMC_RSI_MEASUREMENT_EXTEND
 *          - regs.a1 = index
 *          - regs.a2 = size
 *          - regs.a3~: measurement (最多 64 字节)
 *
 * 头文件
 * #include<linux/tsm.h>
 */

#define ATTEST_DRIVER_NAME "/dev/attest"
#define CMD_IOC_MAGIC 'A'
#define WORD_SIZE 8
#define CMD_IOC_1 _IOWR(CMD_IOC_MAGIC, 1, struct IOC_ARGS_ATTEST)
#define REM_INDEX 3

/*
 * IOC_ARGS_ATTEST 与内核 tsm_measurement 字段映射:
 *   index  (uint32_t)      <-> tsm_measurement.index     (unsigned int)
 *   size   (uint32_t)      <-> tsm_measurement.value_len (size_t)
 *   words  (uint64_t[8])   <-> tsm_measurement.value     (u8[64])
 *
 * words[8] = 64 字节, 与 TSM_MEASUREMENT_MAX_SIZE 一致。
 */
struct IOC_ARGS_ATTEST
{
    uint32_t index;
    uint32_t size;
    uint64_t words[WORD_SIZE];
};

static int measurement_extend(int fd, uint32_t index, uint64_t words[WORD_SIZE])
{
/*
 * 使用内核模块extend rem3
 *
 * /dev/attest 驱动侧 ioctl 适配:
 *   驱动收到 IOC_ARGS_ATTEST 后, 构造 tsm_measurement,
 *   通过 include/linux/tsm.h 中的 measurement_extend 函数指针调用,
 *   避免手动 extend。
 *
 *   struct IOC_ARGS_ATTEST *args = (struct IOC_ARGS_ATTEST *)arg;
 *
 *   struct tsm_measurement m;
 *   m.index = args->index;
 *   m.value_len = args->size;
 *   memcpy(m.value, args->words, args->size);
 *
 *   int rc = ops->measurement_extend(&m, NULL);
 *   if (rc)
 *       return rc;
 *   return 0;
 */
    struct IOC_ARGS_ATTEST args;
    args.index = index;
    args.size = WORD_SIZE * sizeof(uint64_t);
    for (int i = 0; i < WORD_SIZE; i++)
        args.words[i] = words[i];

    return ioctl(fd, CMD_IOC_1, &args);
}

/* 将十六进制字符串解析为字节数组，不足64字节的右侧补零 */
static int parse_hex(const char *hex, uint8_t out[WORD_SIZE * sizeof(uint64_t)])
{
    size_t hex_len = strlen(hex);
    if (hex_len > WORD_SIZE * sizeof(uint64_t) * 2)
    {
        fprintf(stderr, "error: hex data exceeds 64 bytes\n");
        return -1;
    }
    memset(out, 0, WORD_SIZE * sizeof(uint64_t));
    for (size_t i = 0; i < hex_len / 2; i++)
    {
        unsigned int byte;
        if (sscanf(hex + 2 * i, "%2x", &byte) != 1)
        {
            fprintf(stderr, "error: invalid hex character at position %zu\n", 2 * i);
            return -1;
        }
        out[i] = (uint8_t)byte;
    }
    return 0;
}

int main(int argc, char *argv[])
{
    if (argc != 2)
    {
        fprintf(stderr, "Usage: %s <hex_data>\n", argv[0]);
        fprintf(stderr, "  hex_data : up to 128 hex characters (64 bytes), padded with zeros\n");
        return EXIT_FAILURE;
    }

    uint8_t raw[WORD_SIZE * sizeof(uint64_t)];
    if (parse_hex(argv[1], raw) != 0)
        return EXIT_FAILURE;

    uint64_t words[WORD_SIZE];
    memcpy(words, raw, sizeof(words));

    int fd = open(ATTEST_DRIVER_NAME, O_RDWR);
    if (fd < 0)
    {
        perror("open " ATTEST_DRIVER_NAME);
        return EXIT_FAILURE;
    }

    int res = measurement_extend(fd, REM_INDEX, words);
    close(fd);

    if (res != 0)
    {
        perror("ioctl measurement_extend");
        return EXIT_FAILURE;
    }

    return EXIT_SUCCESS;
}
