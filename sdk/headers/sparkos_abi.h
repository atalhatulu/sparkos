/*
 * SparkOS Official Userspace C ABI Header
 * Copyright (c) 2026 SparkOS Team
 */

#ifndef SPARKOS_ABI_H
#define SPARKOS_ABI_H

#include <stdint.h>

/* Canonical Syscall Numbers */
#define SYS_READ               0
#define SYS_EXIT               1
#define SYS_OPEN               2
#define SYS_CLOSE              3
#define SYS_WRITE              4
#define SYS_EXEC               7
#define SYS_WAITPID            8
#define SYS_YIELD              9

#define SYS_SOCKET             10
#define SYS_CONNECT            11
#define SYS_SEND               12
#define SYS_RECV               13

#define SYS_IPC_SEND           20
#define SYS_IPC_RECV           21
#define SYS_IOPERM             22
#define SYS_IPC_TRY_RECV       23
#define SYS_IPC_CREATE_ENDPOINT 24
#define SYS_IPC_BIND_IRQ       25
#define SYS_MAP_DMA            26
#define SYS_IPC_CANCEL         29
#define SYS_IPC_CREATE_SLOT    30

#define SYS_CREATE_SURFACE     31
#define SYS_PRESENT_SURFACE    32
#define SYS_DESTROY_SURFACE    33

/* Standard Error Codes */
#define SPARK_OK               0
#define SPARK_EINVAL          -1
#define SPARK_ENOENT          -2
#define SPARK_EPERM           -3
#define SPARK_EBADF           -4
#define SPARK_EAGAIN          -5
#define SPARK_ENOMEM          -6
#define SPARK_EEXIST          -7
#define SPARK_ECONNREFUSED    -8
#define SPARK_ETIMEDOUT       -9

#endif /* SPARKOS_ABI_H */
