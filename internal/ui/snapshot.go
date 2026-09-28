package ui

// #cgo pkg-config: gtk4
// #include <gtk/gtk.h>
//
// static int snapshot_widget_png(GtkWidget *widget, const char *path) {
//   int w = gtk_widget_get_width(widget), h = gtk_widget_get_height(widget);
//   GdkPaintable *p = gtk_widget_paintable_new(widget);
//   GtkSnapshot *s = gtk_snapshot_new();
//   gdk_paintable_snapshot(p, GDK_SNAPSHOT(s), w, h);
//   GskRenderNode *node = gtk_snapshot_free_to_node(s);
//   int ok = 0;
//   if (node) {
//     GskRenderer *r = gtk_native_get_renderer(gtk_widget_get_native(widget));
//     graphene_rect_t vp = GRAPHENE_RECT_INIT(0, 0, w, h);
//     GdkTexture *t = gsk_renderer_render_texture(r, node, &vp);
//     ok = gdk_texture_save_to_png(t, path);
//     g_object_unref(t);
//     gsk_render_node_unref(node);
//   }
//   g_object_unref(p);
//   return ok;
// }
import "C"

import (
	"fmt"
	"unsafe"

	"github.com/diamondburned/gotk4/pkg/gtk/v4"
)

func snapshotWidgetPNG(widget gtk.Widgetter, path string) error {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	ptr := (*C.GtkWidget)(unsafe.Pointer(gtk.BaseWidget(widget).Object.Native()))
	if C.snapshot_widget_png(ptr, cpath) == 0 {
		return fmt.Errorf("snapshot failed")
	}
	return nil
}
