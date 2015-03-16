/*
 * @(#)FCGIInputStream.java
 *
 *      FastCGi compatibility package Interface
 *
 *
 *  Copyright (c) 1996 Open Market, Inc.
 *
 * See the file "LICENSE.TERMS" for information on usage and redistribution
 * of this file, and for a DISCLAIMER OF ALL WARRANTIES.
 *
 * $Id: FCGIInputStream.java,v 1.4 2000/03/21 12:12:25 robs Exp $
 */
package com.fastcgi;

import java.io.*;

/**
 * This stream manages buffered reads of FCGI messages.
 */
public class FCGIInputStream extends InputStream
{
    private static final String RCSID = "$Id: FCGIInputStream.java,v 1.4 2000/03/21 12:12:25 robs Exp $";

    /* Stream vars */

    public int rdNext;
    public int stop;
    public boolean isClosed;

    /* require methods to set, get and clear */
    private int errno;
    private Exception errex;

    /* data vars */

    public byte buff[];
    public int buffLen;
    public int buffStop;
    public int type;
    public int contentLen;
    public int paddingLen;
    public boolean skip;
    public boolean eorStop;
    public FCGIRequest request;

    public InputStream in;


    /**
    * Creates a new input stream to manage fcgi prototcol stuff
    * @param in the input stream  bufLen  length of buffer streamType
    */
    public FCGIInputStream(FileInputStream inStream, int bufLen,
        int streamType,
        FCGIRequest inReq) {

        in = inStream;
        buffLen = Math.min(bufLen,FCGIGlobalDefs.def_FCGIMaxLen);
        buff = new byte[buffLen];
        type = streamType;
        stop = rdNext = buffStop = 0;
        isClosed = false;
        contentLen = 0;
        paddingLen = 0;
        skip = false;
        eorStop = false;
        request = inReq;

    }
    /**
    * Reads a byte of data. This method will block if no input is
    * available.
    * @return  the byte read, or -1 if the end of the
    *      stream is reached.
    * @exception IOException If an I/O error has occurred.
    */
    public int read() throws IOException {
        if (rdNext != stop) {
            return buff[rdNext++] & 0xff;
        }
        if (isClosed){
            return -1;
        }
        fill();
        if (rdNext != stop){
            return buff[rdNext++] & 0xff;
        }
        return -1;
    }
    /**
    * Reads into an array of bytes.  This method will
    * block until some input is available.
    * @param b the buffer into which the data is read
    * @return  the actual number of bytes read, -1 is
    *      returned when the end of the stream is reached.
    * @exception IOException If an I/O error has occurred.
    */
    public int read(byte b[]) throws IOException {
        return read(b, 0, b.length);
    }

    /**
    * Reads into an array of bytes.
    * Blocks until some input is available.
    * @param b the buffer into which the data is read
    * @param off the start offset of the data
    * @param len the maximum number of bytes read
    * @return  the actual number of bytes read, -1 is
    *      returned when the end of the stream is reached.
    * @exception IOException If an I/O error has occurred.
    */
    public int read(byte b[], int off, int len) throws IOException {
        int m, bytesMoved;

        if (len <= 0){
            return 0;
        }
        /*
        *Fast path: len bytes already available.
        */

        if (len <= stop - rdNext){
            System.arraycopy(buff, rdNext, b, off, len);
            rdNext += len;
            return len;
        }
        /*
        *General case: stream is closed or fill needs to be called
        */
        bytesMoved = 0;
        for(;;){
            if (rdNext != stop){
                m = Math.min(len - bytesMoved, stop - rdNext);
                System.arraycopy(buff, rdNext, b, off, m);
                bytesMoved += m;
                rdNext += m;
                if (bytesMoved == len)
                    return bytesMoved;
                off += m;
            }
            if (isClosed){
                return bytesMoved;
            }
            fill();

        }
    }
    /**
    * Reads into an array of bytes.  This method will
    * block until some input is available.
    * @param b the buffer into which the data is read
    * @param off the start offset of the data
    * @param len the maximum number of bytes read
    * @return  the actual number of bytes read, -1 is
    *      returned when the end of the stream is reached.
    * @exception IOException If an I/O error has occurred.
    */
    public void  fill() throws IOException {
        byte[] headerBuf = new byte[FCGIGlobalDefs.def_FCGIHeaderLen];
        int headerLen = 0;
        int status = 0;
        int count = 0;
        for(;;) {
            /*
            * If buffer is empty, do a read
            */
            if (rdNext == buffStop) {
                try {
                    count = in.read(buff, 0, buffLen);
                } catch (IOException e) {
                    setException(e);
                    return;
                }
                if (count == 0) {
                    setFCGIError(FCGIGlobalDefs.def_FCGIProtocolError);
                    return;
                }
                rdNext = 0;
                buffStop = count;       // 1 more than we read
            }
            /* Now buf is not empty: If the current record contains more content
             * bytes, deliver all that are present in buff to callers buffer
             * unless he asked for less than we have, in which case give him less
             */
            if (contentLen > 0) {
                count = Math.min(contentLen, buffStop - rdNext);
                contentLen -= count;
                if (!skip) {
                    stop = rdNext + count;
                    return;
                }
                else {
                    rdNext += count;
                    if (contentLen > 0) {
                        continue;
                    }
                    else {
                        skip = false;
                    }
                }
            }
            /* Content has been consumed by client.
             * If record was padded, skip over padding
             */
            if (paddingLen > 0) {
                count = Math.min(paddingLen, buffStop - rdNext);
                paddingLen -= count;
                rdNext += count;
                if (paddingLen > 0) {
                    continue;    // more padding to read
                }
            }
            /* All done with current record, including the padding.
             * If we are in a recursive call from Process Header, deliver EOF
             */
            if (eorStop){
                stop = rdNext;
                isClosed = true;
                return;
            }
            /*
             * Fill header with bytes from input buffer - get the whole header.
             */
            count = Math.min(headerBuf.length - headerLen, buffStop - rdNext);
            System.arraycopy(buff,rdNext, headerBuf, headerLen, count);
            headerLen += count;
            rdNext  += count;
            if (headerLen < headerBuf.length) {
                continue;
            }
            headerLen = 0;
            /*
             * Interperet the header. eorStop prevents ProcessHeader from
             * reading past the end of record when using stream to read content
             */
            eorStop = true;
            stop = rdNext;
            status = 0;
            status = new FCGIMessage(this).processHeader(headerBuf);
            eorStop = false;
            isClosed = false;
            switch (status){
            case FCGIGlobalDefs.def_FCGIStreamRecord:
                if (contentLen == 0) {
                    stop = rdNext;
                    isClosed = true;
                    return;
                }
                break;
            case FCGIGlobalDefs.def_FCGISkip:
                skip = true;
                break;
            case FCGIGlobalDefs.def_FCGIBeginRecord:
                /*
                * If this header marked the beginning of a new
                * request, return role info to caller
                */
                return;
            case FCGIGlobalDefs.def_FCGIMgmtRecord:
                break;
            default:
                /*
                * ASSERT
                */
                setFCGIError(status);
                return;

            }
        }
    }

    /**
     * Skips n bytes of input.
     * @param n the number of bytes to be skipped
     * @return  the actual number of bytes skipped.
     * @exception IOException If an I/O error has occurred.
     */
    public long skip(long n) throws IOException {
        byte data[] = new byte[(int)n];
        return in.read(data);
    }

    /*
     * An FCGI error has occurred. Save the error code in the stream
     * for diagnostic purposes and set the stream state so that
     * reads return EOF
     */
    public void setFCGIError(int errnum) {
        /*
        * Preserve only the first error.
        */
        if(errno == 0) {
            errno = errnum;
        }
        isClosed = true;
    }
    /*
    * An Exception has occurred. Save the Exception in the stream
    * for diagnostic purposes and set the stream state so that
    * reads return EOF
    */
    public void setException(Exception errexpt) {
        /*
        * Preserve only the first error.
        */
        if(errex == null) {
            errex = errexpt;
        }
        isClosed = true;
    }

    /*
    * Clear the stream error code and end-of-file indication.
    */
    public void clearFCGIError() {
        errno = 0;
        /*
        * isClosed = false;
        * XXX: should clear isClosed but work is needed to make it safe
        * to do so.
        */
    }
    /*
    * Clear the stream error code and end-of-file indication.
    */
    public void clearException() {
        errex = null;
        /*
        * isClosed = false;
        * XXX: should clear isClosed but work is needed to make it safe
        * to do so.
        */
    }

    /*
    * accessor method since var is private
    */
    public int getFCGIError() {
        return errno;
    }
    /*
    * accessor method since var is private
    */
    public Exception getException() {
        return errex;
    }
    /*
    * Re-initializes the stream to read data of the specified type.
    */
    public void setReaderType(int streamType) {

        type = streamType;
        eorStop = false;
        skip = false;
        contentLen = 0;
        paddingLen = 0;
        stop = rdNext;
        isClosed = false;
    }

    /*
    * Close the stream. This method does not really exist for BufferedInputStream in java,
    * but is implemented here for compatibility with the FCGI structures being used. It
    * doent really throw any IOExceptions either, but that's there for compatiblity with
    * the InputStreamInterface.
    */
    public void close() throws IOException{
        isClosed = true;
        stop = rdNext;
    }

    /*
    * Returns the number of bytes that can be read without blocking.
    */

    public int available() throws IOException {
        return stop - rdNext + in.available();
    }

}
