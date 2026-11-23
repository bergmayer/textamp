#ifndef TEXTAMP_CORE_H
#define TEXTAMP_CORE_H
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
typedef struct TextampClient TextampClient;
typedef struct { uint8_t *data; size_t length; } TextampBytes;
TextampClient *textamp_start(const char *options, char **error, bool (*output_session)(bool active));
char *textamp_send(const TextampClient *client, const char *command);
TextampBytes textamp_drain(const TextampClient *client);
TextampBytes textamp_asset(const TextampClient *client, const char *id);
TextampBytes textamp_character_art(const uint8_t *data, size_t length, const char *mode);
TextampBytes textamp_activity_history(void);
void textamp_record_error(const char *message);
void textamp_stop(TextampClient *client);
void textamp_bytes_free(TextampBytes bytes);
void textamp_string_free(char *string);
#endif
