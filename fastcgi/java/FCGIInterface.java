/*
 * @(#)FCGIInterface.java
 *
 *
 *      FastCGi compatibility package Interface
 *
 *
 *  Copyright (c) 1996 Open Market, Inc.
 *
 * See the file "LICENSE.TERMS" for information on usage and redistribution
 * of this file, and for a DISCLAIMER OF ALL WARRANTIES.
 *
 * $Id: FCGIInterface.java,v 1.4 2000/03/27 15:37:25 robs Exp $
 */
package com.fastcgi;

import java.net.*;
import java.io.*;
import java.util.Properties;

/*
 * This is the FastCGI interface that the application calls to communicate with the
 * FastCGI web server. This version is single threaded, and handles one request at
 * a time, which is why we can have a static variable for it.
 */
public class FCGIInterface 
{
    private static final String RCSID = "$Id: FCGIInterface.java,v 1.4 2000/03/27 15:37:25 robs Exp $";

    /*
    * Class variables
    */
    public static FCGIRequest request = null;
    public static boolean acceptCalled = false;
    public static boolean isFCGI = true;
    public static Properties startupProps;
    public static ServerSocket srvSocket;

    /*
    * Accepts a new request from the HTTP server and creates
    * a conventional execution environment for the request.
    * If the application was invoked as a FastCGI server,
    * the first call to FCGIaccept indicates that the application
    * has completed its initialization and is ready to accept
    * a request.  Subsequent calls to FCGI_accept indicate that
    * the application has completed its processing of the
    * current request and is ready to accept a new request.
    * If the application was invoked as a CGI program, the first
    * call to FCGIaccept is essentially a no-op and the second
    * call returns EOF (-1) as does an error. Application should exit.
    *
    * If the application was invoked as a FastCGI server,
    * and this is not the first call to this procedure,
    * FCGIaccept first flushes any buffered output to the HTTP server.
    *
    * On every call, FCGIaccept accepts the new request and
    * reads the FCGI_PARAMS stream into System.props. It also creates
    * streams that understand FastCGI protocol and take input from
    * the HTTP server send output and error output to the HTTP server,
    * and assigns these new streams to System.in, System.out and
    * System.err respectively.
    *
    * For now, we will just return an int to the caller, which is why
    * this method catches, but doen't throw Exceptions.
    *
    */
    public int FCGIaccept() {
        int acceptResult = 0;

        /*
         * If first call, mark it and if fcgi save original system properties,
         * If not first call, and  we are cgi, we should be gone.
         */
        if (!acceptCalled){
            isFCGI = System.getProperties().containsKey("FCGI_PORT");
            acceptCalled = true;
            if (isFCGI) {
                /*
                 * save original system properties (nonrequest)
                 * and get a server socket
                 */
                startupProps = new Properties(System.getProperties());
                String str =
                    new String(System.getProperty("FCGI_PORT"));
                if (str.length() <= 0) {
                    return -1;
                }
                int portNum = Integer.parseInt(str);

                try {
                    srvSocket = new ServerSocket(portNum);
                } catch (IOException e) {
                    if (request != null)
                    {
                        request.socket = null;
                    }
                    srvSocket = null;
                    request = null;
                    return -1;
                }
            }
        }
        else {
            if (!isFCGI){
                return -1;
            }
        }
        /*
         * If we are cgi, just leave everything as is, otherwise set up env
         */
        if (isFCGI){
            try {
                acceptResult = FCGIAccept();
            } catch (IOException e) {
                return -1;
            }
            if (acceptResult < 0){
                return -1;
            }

            /*
            * redirect stdin, stdout and stderr to fcgi socket
            */
            System.setIn(new BufferedInputStream(request.inStream, 8192));
            System.setOut(new PrintStream(new BufferedOutputStream(
                request.outStream, 8192)));
            System.setErr(new PrintStream(new BufferedOutputStream(
                request.errStream, 512)));
            System.setProperties(request.params);
        }
        return 0;
    }

    /*
     * Accepts a new request from the HTTP server.
     * Finishes the request accepted by the previous call
     * to FCGI_Accept. Sets up the FCGI environment and reads
     * saved and per request environmental varaibles into
     * the request object. (This is redundant on System.props
     * as long as we can handle only one request object.)
     */
    int FCGIAccept() throws IOException{

        boolean isNewConnection;
        boolean errCloseEx = false;
        boolean outCloseEx = false;

        if (request != null) {
            /*
             * Complete the previous request
             */
            System.err.close();
            System.out.close();
            boolean prevRequestfailed = (errCloseEx || outCloseEx ||
                request.inStream.getFCGIError() != 0 ||
                request.inStream.getException() != null);
            if (prevRequestfailed || !request.keepConnection ) {
                request.socket.close();
                request.socket = null;
            }
            if (prevRequestfailed) {
                request = null;
                return -1;
            }
        }
        else    {
            /*
             * Get a Request and initialize some variables
             */
            request = new FCGIRequest();
            request.socket = null;
            request.inStream = null;
        }
        isNewConnection = false;

        /*
         * if connection isnt open accept a new connection (blocking)
         */
        for(;;) {
            if (request.socket == null){
                try {
                    request.socket = srvSocket.accept();
                } catch (IOException e) {
                    request.socket = null;
                    request = null;
                    return -1;
                }
                isNewConnection = true;
            }

            /* Try reading from new connection. If the read fails and
             * it was an old connection the web server probably closed it;
             * try making a new connection before giving up
             */
            request.isBeginProcessed = false;
            request.inStream =
                new FCGIInputStream((FileInputStream)request.
                socket.getInputStream(),
                8192, 0, request);
            request.inStream.fill();
            if (request.isBeginProcessed) {
                break;
            }
            request.socket.close();

                request.socket = null;
            if (isNewConnection) {
                return -1;
            }
        }
        /*
        * Set up the objects for the new request
        */
        request.params = new Properties(startupProps);
        switch(request.role) {
        case FCGIGlobalDefs.def_FCGIResponder:
            request.params.put("ROLE","RESPONDER");
            break;
        case FCGIGlobalDefs.def_FCGIAuthorizer:
            request.params.put("ROLE", "AUTHORIZER");
            break;
        case FCGIGlobalDefs.def_FCGIFilter:
            request.params.put("ROLE", "FILTER");
            break;
        default:
            return -1;
        }
        request.inStream.setReaderType(FCGIGlobalDefs.def_FCGIParams);
        /*
         * read the rest of request parameters
         */
        if (new FCGIMessage(request.inStream).readParams(request.params) < 0) {
            return -1;
        }
        request.inStream.setReaderType(FCGIGlobalDefs.def_FCGIStdin);
        request.outStream
            =  new FCGIOutputStream((FileOutputStream)request.socket.
            getOutputStream(), 8192,
            FCGIGlobalDefs.def_FCGIStdout,request);
        request.errStream
            = new FCGIOutputStream((FileOutputStream)request.socket.
            getOutputStream(), 512,
            FCGIGlobalDefs.def_FCGIStderr,request);
        request.numWriters = 2;
        return 0;
    }
}
