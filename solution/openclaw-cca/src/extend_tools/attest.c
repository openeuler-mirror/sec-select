#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <sys/ioctl.h>

#define ATTEST_DRIVER_NAME "/dev/attest"
#define CMD_IOC_MAGIC 'A'
#define WORD_SIZE 8
#define CMD_IOC_1 _IOWR(CMD_IOC_MAGIC, 1, struct IOC_ARGS_ATTEST)
#define REM_INDEX 3

struct IOC_ARGS_ATTEST
{
    uint32_t index;
    uint32_t size;
    uint64_t words[WORD_SIZE];
};

static int measurement_extend(int fd, uint32_t index, uint64_t words[WORD_SIZE])
{
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
