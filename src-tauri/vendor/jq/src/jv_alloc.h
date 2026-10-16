#ifndef JV_ALLOC_H
#define JV_ALLOC_H

#include <stddef.h>
#include "jv.h"

void* jv_mem_alloc(size_t);
void* jv_mem_alloc_unguarded(size_t);
void* jv_mem_calloc(size_t, size_t);
void* jv_mem_calloc_unguarded(size_t, size_t);
char* jv_mem_strdup(const char *);
char* jv_mem_strdup_unguarded(const char *);
void jv_mem_free(void*);
/* veloterm 修补：__attribute__((warn_unused_result)) 是 GNU 扩展且上游未加
 * 编译器守卫，MSVC 解析该行崩坏并级联报错（C2143/C2091 等假错误）；
 * GNU 工具链（含 MinGW）行为不变：仍带 warn_unused_result。 */
#ifdef __GNUC__
__attribute__((warn_unused_result)) void* jv_mem_realloc(void*, size_t);
#else
void* jv_mem_realloc(void*, size_t);
#endif

#endif
