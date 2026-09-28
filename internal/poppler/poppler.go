// Package poppler is a minimal cgo wrapper around poppler-glib: just enough to
// open a PDF, query page sizes and render pages onto a cairo context.
package poppler

// #cgo pkg-config: poppler-glib cairo cairo-pdf
// #include <stdlib.h>
// #include <poppler.h>
// #include <cairo.h>
// #include <cairo-pdf.h>
import "C"

import (
	"errors"
	"runtime"
	"unsafe"

	"github.com/diamondburned/gotk4/pkg/cairo"
)

// Document is an open PDF. It must only be used from one goroutine at a time;
// open a second Document for background work.
type Document struct {
	p *C.PopplerDocument
}

func Open(path string) (*Document, error) {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))

	var gerr *C.GError
	uri := C.g_filename_to_uri(cpath, nil, &gerr)
	if uri == nil {
		return nil, takeError(gerr)
	}
	defer C.g_free(C.gpointer(uri))

	doc := C.poppler_document_new_from_file(uri, nil, &gerr)
	if doc == nil {
		return nil, takeError(gerr)
	}
	d := &Document{p: doc}
	runtime.SetFinalizer(d, (*Document).Close)
	return d, nil
}

func takeError(gerr *C.GError) error {
	if gerr == nil {
		return errors.New("unknown poppler error")
	}
	defer C.g_error_free(gerr)
	return errors.New(C.GoString(gerr.message))
}

func (d *Document) Close() {
	if d.p != nil {
		C.g_object_unref(C.gpointer(d.p))
		d.p = nil
	}
}

func (d *Document) NPages() int {
	return int(C.poppler_document_get_n_pages(d.p))
}

func (d *Document) page(i int) *C.PopplerPage {
	return C.poppler_document_get_page(d.p, C.int(i))
}

// PageSize returns the page size in PDF points.
func (d *Document) PageSize(i int) (w, h float64) {
	pg := d.page(i)
	if pg == nil {
		return 612, 792
	}
	defer C.g_object_unref(C.gpointer(pg))
	var cw, ch C.double
	C.poppler_page_get_size(pg, &cw, &ch)
	return float64(cw), float64(ch)
}

func ctx(cr *cairo.Context) *C.cairo_t {
	return (*C.cairo_t)(unsafe.Pointer(cr.Native()))
}

// Render draws page i onto cr in PDF point units (the caller sets up scale).
func (d *Document) Render(i int, cr *cairo.Context, forPrinting bool) {
	pg := d.page(i)
	if pg == nil {
		return
	}
	defer C.g_object_unref(C.gpointer(pg))
	if forPrinting {
		C.poppler_page_render_for_printing(pg, ctx(cr))
	} else {
		C.poppler_page_render(pg, ctx(cr))
	}
}

// RenderGray renders page i at the given resolution and returns 8-bit
// luminance (0 = black), plus the bitmap size and pixels-per-point scale.
func (d *Document) RenderGray(i int, dpi float64) (gray []uint8, w, h int, scale float64) {
	pw, ph := d.PageSize(i)
	scale = dpi / 72
	w = max(1, int(pw*scale+0.5))
	h = max(1, int(ph*scale+0.5))
	surf := cairo.CreateImageSurface(cairo.FormatRGB24, w, h)
	cr := cairo.Create(surf)
	cr.SetSourceRGB(1, 1, 1)
	cr.Paint()
	cr.Scale(float64(w)/pw, float64(h)/ph)
	d.Render(i, cr, true)
	surf.Flush()

	data := surf.Data()
	stride := surf.Stride()
	gray = make([]uint8, w*h)
	for y := 0; y < h; y++ {
		row := data[y*stride : y*stride+w*4]
		for x := 0; x < w; x++ {
			b, g, r := uint32(row[x*4]), uint32(row[x*4+1]), uint32(row[x*4+2])
			gray[y*w+x] = uint8((b*29 + g*150 + r*77) >> 8)
		}
	}
	return gray, w, h, float64(w) / pw
}

// SetPDFPageSize changes the size of the next page of a cairo PDF surface.
func SetPDFPageSize(s *cairo.Surface, w, h float64) {
	C.cairo_pdf_surface_set_size((*C.cairo_surface_t)(unsafe.Pointer(s.Native())), C.double(w), C.double(h))
}

// FinishSurface flushes and closes a cairo surface (writes out PDF files).
func FinishSurface(s *cairo.Surface) {
	C.cairo_surface_finish((*C.cairo_surface_t)(unsafe.Pointer(s.Native())))
}
