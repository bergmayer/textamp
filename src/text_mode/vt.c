#include <ghostty/vt.h>
#include <stdlib.h>
#include <string.h>

/* Each handle and its iterators belong to one renderer thread. */
typedef struct {
    GhosttyTerminal terminal;
    GhosttyRenderState render;
    GhosttyRenderStateRowIterator rows;
    GhosttyRenderStateRowCells cells;
    GhosttyRenderStateImageIterator images;
    GhosttyColorRgb foreground, background;
} TextampVt;

typedef struct {
    uint8_t foreground[3], background[3];
    uint16_t flags;
    uint8_t wide, selected;
} TextampVtCell;

typedef struct {
    uint16_t x, y;
    uint8_t visible, blinking, wide_tail, style;
} TextampVtCursor;

void textamp_vt_free(TextampVt *vt) {
    if (!vt) return;
    if (vt->images) ghostty_render_state_image_iterator_free(vt->images);
    if (vt->cells) ghostty_render_state_row_cells_free(vt->cells);
    if (vt->rows) ghostty_render_state_row_iterator_free(vt->rows);
    if (vt->render) ghostty_render_state_free(vt->render);
    if (vt->terminal) ghostty_terminal_free(vt->terminal);
    free(vt);
}

int textamp_vt_png_decoder(GhosttySysDecodePngFn decoder) {
    return ghostty_sys_set(GHOSTTY_SYS_OPT_DECODE_PNG, decoder);
}
uint8_t *textamp_vt_image_alloc(const GhosttyAllocator *allocator, size_t length) {
    return ghostty_alloc(allocator, length);
}

TextampVt *textamp_vt_new(uint16_t cols, uint16_t rows, int *error) {
    TextampVt *vt = calloc(1, sizeof(*vt));
    if (!vt) { *error = GHOSTTY_OUT_OF_MEMORY; return NULL; }
#define CREATE(call) do { *error = (call); if (*error != GHOSTTY_SUCCESS) { textamp_vt_free(vt); return NULL; } } while (0)
    CREATE(ghostty_terminal_new(NULL, &vt->terminal, cols, rows));
    CREATE(ghostty_render_state_new(NULL, &vt->render));
    CREATE(ghostty_render_state_row_iterator_new(NULL, &vt->rows));
    CREATE(ghostty_render_state_row_cells_new(NULL, &vt->cells));
    CREATE(ghostty_render_state_image_iterator_new(NULL, &vt->images));
    size_t storage = 64 * 1024 * 1024;
    CREATE(ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT, &storage));
#undef CREATE
    return vt;
}
int textamp_vt_resize(TextampVt *vt, uint16_t cols, uint16_t rows, uint32_t cw, uint32_t ch) {
    return ghostty_terminal_resize(vt->terminal, cols, rows, cw, ch);
}
void textamp_vt_write(TextampVt *vt, const uint8_t *data, size_t length) {
    ghostty_terminal_vt_write(vt->terminal, data, length);
}
int textamp_vt_update(TextampVt *vt, uint8_t *dirty, TextampVtCursor *cursor) {
    GhosttyResult result = ghostty_render_state_update(vt->render, vt->terminal);
    if (result != GHOSTTY_SUCCESS) return result;
    GhosttyRenderStateDirty changed;
    ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_DIRTY, &changed);
    *dirty = (uint8_t)changed;
    ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_COLOR_FOREGROUND, &vt->foreground);
    ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_COLOR_BACKGROUND, &vt->background);
    GhosttyRenderStateCursor state = GHOSTTY_INIT_SIZED(GhosttyRenderStateCursor);
    ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_CURSOR, &state);
    memset(cursor, 0, sizeof(*cursor));
    if (state.viewport_has_value) {
        cursor->x=state.viewport_x; cursor->y=state.viewport_y;
        cursor->visible=state.visible; cursor->blinking=state.blinking;
        cursor->wide_tail=state.wide_tail; cursor->style=(uint8_t)state.visual_style;
    }
    return ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &vt->rows);
}
int textamp_vt_row(TextampVt *vt, int32_t *y, uint8_t *dirty) {
    if (!ghostty_render_state_row_iterator_next(vt->rows)) return 0;
    bool changed=false;
    ghostty_render_state_row_get(vt->rows, GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y, y);
    ghostty_render_state_row_get(vt->rows, GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY, &changed);
    *dirty=changed;
    bool clean=false;
    ghostty_render_state_row_set(vt->rows,GHOSTTY_RENDER_STATE_ROW_OPT_DIRTY,&clean);
    ghostty_render_state_row_get(vt->rows, GHOSTTY_RENDER_STATE_ROW_DATA_CELLS, &vt->cells);
    return 1;
}
int textamp_vt_cell(TextampVt *vt, uint16_t x, TextampVtCell *out, uint8_t *text, size_t capacity, size_t *length) {
    GhosttyResult result=ghostty_render_state_row_cells_select(vt->cells,x);
    if (result != GHOSTTY_SUCCESS) return result;
    GhosttyStyle style={0}; GhosttyCell raw={0}; GhosttyCellWide wide=GHOSTTY_CELL_WIDE_NARROW;
    GhosttyColorRgb foreground=vt->foreground, background=vt->background;
    bool selected=false;
    ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,&style);
    ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,&raw);
    ghostty_cell_get(raw,GHOSTTY_CELL_DATA_WIDE,&wide);
    ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,&foreground);
    ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,&background);
    ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED,&selected);
    memcpy(out->foreground,&foreground,3); memcpy(out->background,&background,3);
    out->flags=style.flags; out->wide=(uint8_t)wide; out->selected=selected;
    GhosttyBuffer buffer={.ptr=text,.cap=capacity};
    result=ghostty_render_state_row_cells_get(vt->cells,GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,&buffer);
    *length=buffer.len;
    return result;
}
void textamp_vt_finish(TextampVt *vt) {
    GhosttyRenderStateDirty dirty=GHOSTTY_RENDER_STATE_DIRTY_FALSE;
    ghostty_render_state_set(vt->render,GHOSTTY_RENDER_STATE_OPT_DIRTY,&dirty);
}

typedef struct {
    uint32_t id, placement;
    uint64_t generation;
    int32_t column, row, z;
    uint32_t offset_x, offset_y, draw_width, draw_height;
    uint32_t source_x, source_y, source_width, source_height;
    uint32_t width, height;
    uint8_t layer, format;
    const uint8_t *pixels;
    size_t length;
} TextampVtImage;
int textamp_vt_images_start(TextampVt *vt) {
    return ghostty_render_state_get(vt->render,GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR,&vt->images);
}
int textamp_vt_image(TextampVt *vt, TextampVtImage *out) {
    if (!ghostty_render_state_image_iterator_next(vt->images)) return 0;
    memset(out,0,sizeof(*out));
    GhosttyImageLayer layer;
#define IMAGE(key, field) ghostty_render_state_image_get(vt->images,GHOSTTY_RENDER_STATE_IMAGE_DATA_##key,&out->field)
    IMAGE(IMAGE_ID,id); IMAGE(IMAGE_GENERATION,generation); IMAGE(KITTY_PLACEMENT_ID,placement);
    IMAGE(VIEWPORT_COL,column); IMAGE(VIEWPORT_ROW,row); IMAGE(Z,z);
    IMAGE(CELL_OFFSET_X,offset_x); IMAGE(CELL_OFFSET_Y,offset_y);
    IMAGE(PIXEL_WIDTH,draw_width); IMAGE(PIXEL_HEIGHT,draw_height);
    IMAGE(SOURCE_X,source_x); IMAGE(SOURCE_Y,source_y);
    IMAGE(SOURCE_WIDTH,source_width); IMAGE(SOURCE_HEIGHT,source_height);
#undef IMAGE
    ghostty_render_state_image_get(vt->images,GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER,&layer);
    out->layer=(uint8_t)layer;
    GhosttyKittyGraphics storage=NULL;
    if (ghostty_terminal_get(vt->terminal,GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,&storage) != GHOSTTY_SUCCESS) return -1;
    GhosttyKittyGraphicsImage image=ghostty_kitty_graphics_image(storage,out->id);
    if (!image) return -1;
    GhosttyKittyImageFormat format;
    GhosttyString pixels;
    ghostty_kitty_graphics_image_get(image,GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_WIDTH,&out->width);
    ghostty_kitty_graphics_image_get(image,GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_HEIGHT,&out->height);
    ghostty_kitty_graphics_image_get(image,GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_FORMAT,&format);
    if (ghostty_kitty_graphics_image_get(image,GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_PIXELS,&pixels) != GHOSTTY_SUCCESS) return -1;
    out->format=(uint8_t)format; out->pixels=(const uint8_t*)pixels.ptr; out->length=pixels.len;
    return 1;
}
