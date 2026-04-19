#include <stdlib.h>
#include <stdio.h>
#include "utility.h"

void print_vector(const char* name, const unsigned char* v) 
{
  int count;
  printf("%s = \n", name);
  for (count = 0; count < 32; count++)
    printf("%02x ", v[count]);
  printf("\n");
}

void print_bytes(const char* name, const unsigned char* v, int numbytes)
{
  int count;
  printf("%s = \n", name);
  for (count = 0; count < numbytes; count++)
    printf("%02x ", v[count]);
  printf("\n");
}

void print_fe(const char* name, const fe in)
{
  unsigned char bytes[32];
  fe_tobytes(bytes, in);
  print_vector(name, bytes);
}

