
#include "fe.h"
#include "crypto_additions.h"

void fe_montx_to_edy(fe y, const fe u)
{
  /* 
     y = (u - 1) / (u + 1)

     NOTE: u=-1 is converted to y=0 since fe_invert is mod-exp
  */
  fe one, um1, up1;

  fe_1(one);
  fe_sub(um1, u, one);
  fe_add(up1, u, one);
  fe_invert(up1, up1);
  fe_mul(y, um1, up1);
}
