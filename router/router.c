#include "fcgiapp.h"

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>

FCGX_Stream *gOutput;

const char *gMozSearchPath;
const char *gIndexPath;

void
Put(const char *s)
{
    int result;
    if ((result = FCGX_PutStr(s, strlen(s), gOutput)) != strlen(s)) {
	fprintf(stderr, "FastCGI write error %d\n", result);
	exit(1);
    }
}

int
StartsWith(const char *longer, const char *shorter)
{
    while (*longer && *shorter) {
	if (*longer++ != *shorter++) {
	    return 0;
	}
    }
    if (*shorter) {
	return 1;
    }
    return 0;
}

void
GenerateError(const char *error)
{
    Put("Content-type: text/html\r\n"
	"\r\n"
	"<h1>Error</h1>\r\n");
    Put(error);
}

void
GenerateFile(const char *path)
{
    if (strstr(path, "..")) {
	GenerateError("Invalid file path");
	return;
    }

    char filename[1024];
    snprintf(filename, sizeof(filename), "%s/file/%s", gIndexPath, path);

    char template[1024];
    snprintf(template, sizeof(template), "%s/%s", gMozSearchPath, "file-template.html");

    GenerateWithTemplate(template, filename);
}

void
GenerateCrossref(const char *path)
{
    if (strstr(path, "/")) {
	GenerateError("Invalid file path");
	return;
    }

    char filename[1024];
    snprintf(filename, sizeof(filename), "%s/crossref", gIndexPath, path);

    char template[1024];
    snprintf(template, sizeof(template), "%s/%s", gMozSearchPath, "file-template.html");

    GenerateWithTemplate(template, filename);
}

int
main(int argc, char *argv[])
{
    gMozSearchPath = argv[1];
    gIndexPath = argv[2];

    int result = FCGX_Init();
    if (result != 0) {
	fprintf(stderr, "FastCGI initialization error %d\n", result);
	exit(1);
    }

    int sock = FCGX_OpenSocket(":8888", 10);
    if (sock == -1) {
	fprintf(stderr, "FastCGI socket error\n");
	exit(1);
    }

    FCGX_Request request;
    result = FCGX_InitRequest(&request, sock, 0);
    if (result != 0) {
	fprintf(stderr, "FastCGI request initialization error %d\n", result);
	exit(1);
    }

    while ((result = FCGX_Accept_r(&request)) != -1) {
	gOutput = request.out;

	Put("Content-type: text/html\r\n"
	    "\r\n"
	    "<h1>hello!</h1>\r\n");

	char *path = FCGX_GetParam("QUERY_STRING", request.envp);

	if (StartsWith(path, "/file/")) {
	    GenerateFile(path + strlen("/file/"));
	} else if (StartsWith(path, "/crossref/")) {
	    GenerateCrossref(path + strlen("/crossref/"));
	} else {
	    GenerateError("Invalid URL");
	}
    }

    return 0;
}
