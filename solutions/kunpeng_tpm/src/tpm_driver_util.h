#ifndef TPM_DRIVER_UTIL_H
#define TPM_DRIVER_UTIL_H

#include <linux/types.h>

#define ARR_IND_0 0
#define ARR_IND_1 1
#define ARR_IND_2 2
#define ARR_IND_3 3
#define ARR_IND_4 4
#define ARR_IND_5 5

#define BIT_SHIFT_8	8U
#define BIT_SHIFT_16	16U
#define BIT_SHIFT_24	24U
#define BIT_SHIFT_6	6U

#define BYTE_4	4U
#define BYTE_7	7U

#ifndef MIN
#define MIN(x, y) ((x) < (y) ? (x) : (y))
#endif

#ifndef NO_USE_PARAM
#define NO_USE_PARAM(p) ((void)(p))
#endif

#endif
