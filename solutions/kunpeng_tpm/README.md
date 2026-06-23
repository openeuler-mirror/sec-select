# kunpeng tpm
基于kunpeng 920B的TPM SPI功能，以Linux内核模块实现，提供sysfs接口进行交互，实现TPM命令写入，信息读取功能。

## 功能

通过SPI总线与TPM芯片通信，支持TPM芯片状态检测，TPM命令发送与响应接收，软件控制SPI CS选片，满足TPM SPI协议时序要求

## 环境

| 项目 | 说明 |
|------|------|
| 硬件平台 | Kunpeng 920B |


## 目录结构

```
kunpeng_tpm/
├── README.md
└── src
    ├── Makefile
    ├── spi_drv.c			#SPI控制器初始化，释放，数据传输，IOMUX复用，CS选片控制
    ├── spi_drv.h
    ├── test_tpm.c			#模块入口，创建sysfs接口"/sys/kernel/test_tpm/test_tpm"，触发tpm命令传输
    ├── tpm_driver_api.c		#提供接口，读取芯片ID判断TPM是否存在
    ├── tpm_driver_api.h
    ├── tpm_driver_cmd.c		#TPM命令传输协议实现，Locality请求/释放，数据分段发送/接收
    ├── tpm_driver_cmd.h
    ├── tpm_driver_common.c		#提供延时函数
    ├── tpm_driver_common.h
    ├── tpm_driver_gpio.c		#GPIO引脚电平设置与方向配置，用于软件控制SPI CS片选信号
    ├── tpm_driver_gpio.h
    ├── tpm_driver_ops.c		#TPM寄存器单字节读写实现，构造SPI命令帧，发送命令并等ACK，读写1字节
    ├── tpm_driver_ops.h
    ├── tpm_driver_spi_reg_v1.h
    ├── tpm_driver_spi_v1.c		#SPI控制器硬件操作，FIFO状态轮询，8位数据收发，参数配置注册
    ├── tpm_driver_spi_v1.h
    ├── tpm_driver_util.h
    └── tpm_driver_v2_interface.h
```

## 快速开始

### 前置条件

安装依赖包
```bash
yum install -y kernel-devel
```

### 构建与安装

```bash
//下载代码
git clone https://gitcode.com/openeuler/sec-select.git

//编译
cd solution/kunpeng_tpm/src/
make -C /lib/modules/{kernel-path}/build M=/{test-code-path} modules

//编译清理
make clean
```

### 使用

```bash
//插入ko
insmod test_tpm_driver.ko

//触发TPM操作
echo 1 > /sys/kernel/test_tpm/test_tpm

//信息查看
dmesg

//卸载ko
rmmod test_tpm_driver
```

