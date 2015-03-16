# $Id: Makefile.am,v 1.8 2001/12/22 03:16:21 robs Exp $

noinst_PROGRAMS     = authorizer        \
                      echo              \
                      echo-x            \
                      log-dump          \
                      size              \
                      @THREADED@        \
                      @ECHO_CPP@

EXTRA_PROGRAMS = threaded echo-cpp

INCLUDEDIR  = ../include
CPPFLAGS    = @CPPFLAGS@ -I$(top_srcdir)/include

INCLUDE_FILES   = $(INCLUDEDIR)/fastcgi.h    \
                  $(INCLUDEDIR)/fcgiapp.h    \
                  $(INCLUDEDIR)/fcgimisc.h   \
                  $(INCLUDEDIR)/fcgi_stdio.h

LIBDIR      = ../libfcgi
LIBFCGI     = $(LIBDIR)/libfcgi.la

LDADD = $(LIBFCGI)

echo_SOURCES       = $(INCLUDE_FILES) echo.c
echo_x_SOURCES     = $(INCLUDE_FILES) echo-x.c
log_dump_SOURCES   = $(INCLUDE_FILES) log-dump.c
authorizer_SOURCES = $(INCLUDE_FILES) authorizer.c

threaded_SOURCES   = $(INCLUDE_FILES) threaded.c
threaded_CC        = @PTHREAD_CC@
threaded_CFLAGS    = @PTHREAD_CFLAGS@
threaded_LDFLAGS   = @PTHREAD_CFLAGS@ @PTHREAD_LIBS@

echo_cpp_SOURCES = $(INCLUDE_FILES) $(INCLUDEDIR)/fcgio.h echo-cpp.cpp
echo_cpp_LDADD   = $(LIBDIR)/libfcgi++.la

