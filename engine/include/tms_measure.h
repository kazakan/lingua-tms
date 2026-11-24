/*
 * tms_measure.h - 엔진 측정 라이브러리가 제공하는 C 인터페이스 (1차 기준, ABI 1)
 *
 * 자체 엔진의 글자 폭·줄바꿈 코드를 PC용 DLL로 빌드해 이 함수들을 내보낸다.
 * TMS(Rust)는 crates/measure 에서 libloading 으로 DLL을 불러 호출한다.
 *
 * 규칙
 *  - 이 파일이 TMS와 엔진 사이의 계약이다. 구조체나 함수 시그니처를 바꾸면
 *    TMS_MEASURE_ABI_VERSION 을 올리고 crates/measure/src/ffi.rs 도 같이 고친다.
 *  - 줄 정보 버퍼는 호출자(TMS)가 넘긴다. DLL은 메모리를 할당해서 돌려주지 않는다.
 *  - TMS는 측정 호출을 한 스레드에서만 순서대로 한다(엔진이 전역 상태를 써도 안전하도록).
 */
#ifndef TMS_MEASURE_H
#define TMS_MEASURE_H

#include <stddef.h>
#include <stdint.h>

#ifdef _WIN32
#  define TMS_API __declspec(dllexport)
#else
#  define TMS_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define TMS_MEASURE_ABI_VERSION 1

typedef struct tms_font tms_font;

typedef struct {
  float   size_px;         /* 글자 크기(px) */
  float   letter_spacing;  /* 자간(px) */
  float   line_height;     /* 0이면 엔진 기본값 */
  int32_t wrap_mode;       /* 0: 줄바꿈 없음, 1: 엔진 기본 줄바꿈 */
} tms_style;

typedef struct {
  int32_t start;           /* UTF-8 바이트 위치(포함) */
  int32_t end;             /* UTF-8 바이트 위치(제외) */
  float   width;           /* 이 줄의 폭(px) */
} tms_line;

typedef struct {
  float   width;           /* 가장 넓은 줄의 폭 */
  float   height;          /* 전체 높이 */
  int32_t line_count;      /* 실제 줄 수(버퍼 크기보다 많을 수 있음) */
  int32_t missing_glyphs;  /* 폴백까지 뒤져도 없는 글자 수 */
  int32_t word_overflow;   /* 단어 하나가 박스보다 넓어 넘친 경우 1 */
} tms_result;

TMS_API uint32_t    tms_abi_version(void);
TMS_API const char* tms_engine_version(void);   /* NUL 종료 문자열, DLL이 소유 */

TMS_API tms_font*   tms_font_load(const uint8_t* ttf, size_t len);  /* .ttf 바이트. 실패 시 NULL */
TMS_API void        tms_font_free(tms_font* font);

/*
 * 반환값 0 = 성공, 그 외 = 엔진 오류 코드.
 * box_width 가 0이면 줄바꿈 없이 한 줄로 측정한다.
 * line_count 가 max_lines 보다 크면 lines 에는 앞의 max_lines 개만 채운다.
 */
TMS_API int32_t tms_layout(const tms_font* const* fonts, size_t font_count, /* 폴백 순서 */
                           const tms_style* style,
                           const char* text_utf8,                          /* NUL 종료 */
                           float box_width,
                           tms_line* lines, size_t max_lines,
                           tms_result* out);

#ifdef __cplusplus
}
#endif

#endif /* TMS_MEASURE_H */
