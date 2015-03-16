#include <stdlib.h>
#include "fcgiapp.h"

int main(void)
{
  int i,scale;
  char* pathInfo;
  FCGX_Stream *in, *out, *err;
  FCGX_ParamArray envp;

  while (FCGX_Accept(&in, &out, &err, &envp) >= 0) 
  {
    FCGX_FPrintF(out,"Content-type: text/plain\r\n\r\n");      
    
    scale = 0;
    
    pathInfo = FCGX_GetParam("PATH_INFO",envp);
    
    if (pathInfo)
      scale = atoi(pathInfo+1);

    if (scale == 0)
      scale = 500;
 
    FCGX_FPrintF(out,"Dumping %6d Bytes ...\n", scale);

    scale = (scale-26)/80;

    for (i=0;i<scale;i++)
    {
      /* each line has 80 character */
      int rv = FCGX_FPrintF(out,"%4d:12345679890123456798901234567989012345679890123456798901234567989012345679890123\n",i);
      if (rv <= 0)
      {
          FCGX_FPrintF(out, "FCGX_FPrintF() failed..");
          break;
      }
    }
  }
  return 0;
}
