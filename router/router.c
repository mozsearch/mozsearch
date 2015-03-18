#include "fcgiapp.h"

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>

FCGX_Stream *gOutput;

const char *gMozSearchPath;
const char *gIndexPath;

char *gCrossRefText;
char **gCrossRefSymbols;
char **gCrossRefResults;
size_t gNumCrossrefs;

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
    if (!*shorter) {
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

char *
ReadFile(const char *filename, size_t *len)
{
    FILE *fp = fopen(filename, "r");
    if (!fp) {
	printf("Not found: %s\n", filename);
	return 0;
    }

    fseek(fp, 0, SEEK_END);
    long size = ftell(fp);
    fseek(fp, 0, SEEK_SET);

    char *result = (char *)malloc(size);

    char *p = result;
    long remaining = size;
    while (remaining) {
	size_t rv = fread(result, 1, remaining, fp);
	if (!rv) {
	    free(result);
	    return 0;
	}
	remaining -= rv;
	p += rv;
    }
    fclose(fp);

    *len = size;

    return result;
}

void
ReadCrossrefs()
{
    char filename[1024];
    snprintf(filename, sizeof(filename), "%s/crossref", gIndexPath);

    size_t size;
    gCrossRefText = ReadFile(filename, &size);
    if (!gCrossRefText) {
	fprintf(stderr, "Unable to read crossref file %s\n", filename);
	exit(1);
    }

    char *p = gCrossRefText;
    size_t numLines = 0;
    while (*p) {
	if (*p == '\n') {
	    numLines++;
	}
	p++;
    }

    gCrossRefSymbols = malloc(sizeof(char *) * (numLines/2 + 1));
    gCrossRefResults = malloc(sizeof(char *) * (numLines/2 + 1));

    p = gCrossRefText;
    int i;
    for (i = 0; ; i++) {
	char *eol = strchr(p, '\n');
	if (!eol) {
	    eol = gCrossRefText + size;
	    if (eol == p) {
		break;
	    }
	}

	if (i & 1) {
	    gCrossRefResults[i / 2] = p;
	} else {
	    gCrossRefSymbols[i / 2] = p;
	}

	if (*eol == '\n') {
	    *eol = 0;
	    p = eol + 1;
	} else {
	    break;
	}
    }

    gNumCrossrefs = i / 2;
}

const char *
FindCrossref(const char *symbol)
{
    size_t i;
    for (i = 0; i < gNumCrossrefs; i++) {
	if (strcmp(gCrossRefSymbols[i], symbol) == 0) {
	    return gCrossRefResults[i];
	}
    }
    return 0;
}

void
GenerateWithTemplate(const char *templateFile, const char *body, size_t bodySize)
{
    size_t templateSize;

    char *template = ReadFile(templateFile, &templateSize);
    if (!template) {
	fprintf(stderr, "Unable to read template file %s\n", templateFile);
	exit(1);
    }

    char *marker = strstr(template, "{{BODY}}");
    if (!marker) {
	free(template);
	fprintf(stderr, "Template does not contain {{BODY}}\n");
	exit(1);
    }

    size_t prefixLen = marker - template;
    size_t markerLen = strlen("{{BODY}}");
    size_t suffixLen = templateSize - prefixLen - markerLen;

    Put("Content-type: text/html\r\n\r\n");

    FCGX_PutStr(template, prefixLen, gOutput);
    FCGX_PutStr(body, bodySize, gOutput);
    FCGX_PutStr(marker + markerLen, suffixLen, gOutput);

    free(template);
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

    size_t bodySize;
    char *body = ReadFile(filename, &bodySize);
    if (!body) {
	GenerateError("Invalid filename");
	return;
    }

    char template[1024];
    snprintf(template, sizeof(template), "%s/%s", gMozSearchPath, "file-template.html");

    GenerateWithTemplate(template, body, bodySize);

    free(body);
}

void
GenerateCrossref(const char *symbol)
{
    char template[1024];
    snprintf(template, sizeof(template), "%s/%s", gMozSearchPath, "crossref-template.html");

    const char *crossref = FindCrossref(symbol);
    if (!crossref) {
	GenerateError("Invalid symbol ID");
	return;
    }

    GenerateWithTemplate(template, crossref, strlen(crossref));
}

void
ReplaceHash(char *path)
{
    char *p = path, *q = path;
    while (*p) {
	if (p[0] == '%' && p[1] == '2' && p[2] == '3') {
	    *q++ = '#';
	    p += 3;
	} else {
	    *q++ = *p++;
	}
    }
    *q = 0;
}

int
main(int argc, char *argv[])
{
    if (argc != 3) {
	fprintf(stderr, "usage: %s <mozsearch-path> <index-path>\n", argv[0]);
	exit(1);
    }

    gMozSearchPath = argv[1];
    gIndexPath = argv[2];

    ReadCrossrefs();

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

	char *path = FCGX_GetParam("QUERY_STRING", request.envp);
	if (!path) {
	    GenerateError("No path?");
	    continue;
	}
	ReplaceHash(path);
	printf("PATH '%s'\n", path);

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
