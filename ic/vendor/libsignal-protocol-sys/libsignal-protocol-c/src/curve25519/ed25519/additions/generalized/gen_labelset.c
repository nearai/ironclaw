#include <stdlib.h>
#include <string.h>
#include "gen_labelset.h"
#include "gen_constants.h"

const unsigned char B_bytes[] = {
  0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
  0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
  0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
  0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
};

unsigned char* buffer_add(unsigned char* bufptr, const unsigned char* bufend,
                          const unsigned char* in, const unsigned long in_len)
{
  unsigned long count = 0;

  if (bufptr == NULL || bufend == NULL || bufptr > bufend)
    return NULL;
  if (in == NULL && in_len != 0)
    return NULL;
  if (bufend - bufptr < in_len)
    return NULL;

  for (count=0; count < in_len; count++) {
    if (bufptr >= bufend)
      return NULL;
    *bufptr++ = *in++;
  }
  return bufptr;
}

unsigned char* buffer_pad(const unsigned char* buf, unsigned char* bufptr, const unsigned char* bufend)
{
  unsigned long count = 0;
  unsigned long pad_len = 0;

  if (buf == NULL || bufptr == NULL || bufend == NULL || bufptr >= bufend || bufptr < buf)
    return NULL;

  pad_len = (BLOCKLEN - ((bufptr-buf) % BLOCKLEN)) % BLOCKLEN;
  if (bufend - bufptr < pad_len)
    return NULL;

  for (count=0; count < pad_len; count++) {
    if (bufptr >= bufend)
      return NULL;
    *bufptr++ = 0;
  }
  return bufptr;
}

int labelset_new(unsigned char* labelset, unsigned long* labelset_len, const unsigned long labelset_maxlen,
                 const unsigned char* protocol_name, const unsigned char protocol_name_len,
                 const unsigned char* customization_label, const unsigned char customization_label_len)
{
  unsigned char* bufptr;

  *labelset_len = 0;
  if (labelset == NULL)
    return -1;
  if (labelset_len == NULL)
    return -1;
  if (labelset_maxlen > LABELSETMAXLEN)
    return -1;
  if (labelset_maxlen < 3 + protocol_name_len + customization_label_len)
    return -1;
  if (protocol_name == NULL && protocol_name_len != 0)
    return -1;
  if (customization_label == NULL && customization_label_len != 0)
    return -1;
  if (protocol_name_len > LABELMAXLEN)
    return -1;
  if (customization_label_len > LABELMAXLEN)
    return -1;

  bufptr = labelset;
  *bufptr++ = 2;
  *bufptr++ = protocol_name_len;
  bufptr = buffer_add(bufptr, labelset + labelset_maxlen, protocol_name, protocol_name_len); 
  if (bufptr != NULL && bufptr < labelset + labelset_maxlen)
    *bufptr++ = customization_label_len;
  bufptr = buffer_add(bufptr, labelset + labelset_maxlen, 
                      customization_label, customization_label_len);

  if (bufptr != NULL && bufptr - labelset == 3 + protocol_name_len + customization_label_len) {
    *labelset_len = bufptr - labelset;
    return 0;
  }
  return -1;
}


int labelset_add(unsigned char* labelset, unsigned long* labelset_len, const unsigned long labelset_maxlen,
              const unsigned char* label, const unsigned char label_len)
{
  unsigned char* bufptr;
  if (labelset_len == NULL)
    return -1;
  if (*labelset_len > LABELSETMAXLEN || labelset_maxlen > LABELSETMAXLEN)
    return -1;
  if (*labelset_len >= labelset_maxlen || *labelset_len + label_len + 1 > labelset_maxlen)
    return -1;
  if (*labelset_len < 3 || labelset_maxlen < 4)
    return -1;
  if (label_len > LABELMAXLEN)
    return -1;

  labelset[0]++;
  labelset[*labelset_len] = label_len;
  bufptr = labelset + *labelset_len + 1;
  bufptr = buffer_add(bufptr, labelset + labelset_maxlen, label, label_len);
  if (bufptr == NULL)
    return -1;
  if (bufptr - labelset >= labelset_maxlen)
    return -1;
  if (bufptr - labelset != *labelset_len + 1 + label_len)
    return -1;

  *labelset_len += (1 + label_len);
  return 0;
}

int labelset_validate(const unsigned char* labelset, const unsigned long labelset_len)
{
  unsigned char num_labels = 0;
  unsigned char count = 0;
  unsigned long offset = 0;
  unsigned char label_len = 0;

  if (labelset == NULL)
    return -1;
  if (labelset_len < 3 || labelset_len > LABELSETMAXLEN)
    return -1;

  num_labels = labelset[0];
  offset = 1;
  for (count = 0; count < num_labels; count++) {
    label_len = labelset[offset];
    if (label_len > LABELMAXLEN)
      return -1;
    offset += 1 + label_len;
    if (offset > labelset_len)
      return -1;
  }
  if (offset != labelset_len)
    return -1;
  return 0;
}

int labelset_is_empty(const unsigned char* labelset, const unsigned long labelset_len)
{
  if (labelset_len != 3)
    return 0;
  return 1;
}

