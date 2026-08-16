#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include "xodus_utils.h"
#include <stdlib.h>
#include <stdio.h>

int main() {
    struct shim_channel channel;
    // HRESULT hr = shim_start(&channel, L"PROXY_PATH");
    // printf("shim_start: %x\n", (int)hr);
    HRESULT hr = shim_connect(&channel);
    fprintf(stderr, "shim_connect: %x\n", (int)hr);

    UINT16 respType;
    char *resp;
    UINT16 respLen;
    hr = shim_call(&channel, PING_REQUEST, "{}", 2, &respType, &resp, &respLen);

    fprintf(stderr, "shim_call: %x type %d len %d:\n%.*s\n", (int)hr, (int)respType, (int)respLen, (int)respLen, resp);

    const char *msg = "<?xml version=\"1.0\"?><MSATokenRequest><ClientId>0000000048183522</ClientId></MSATokenRequest>";
    hr = shim_call(&channel, MSA_TOKEN_REQUEST, msg, strlen(msg), &respType, &resp, &respLen);

    fprintf(stderr, "shim_call: %x type %d len %d\n%.*s\n", (int)hr, (int)respType, (int)respLen, (int)respLen, resp);

    hr = shim_call(&channel, PING_REQUEST, "{2}", 3, &respType, &resp, &respLen);

    fprintf(stderr, "shim_call: %x type %d len %d:\n%.*s\n", (int)hr, (int)respType, (int)respLen, (int)respLen, resp);


    msg = "<?xml version=\"1.0\"?><MSATokenRequest><ClientId>0000000048183522</ClientId></MSATokenRequest>";
    hr = shim_call(&channel, MSA_TOKEN_REQUEST, msg, strlen(msg), &respType, &resp, &respLen);

    fprintf(stderr, "shim_call: %x type %d len %d\n%.*s\n", (int)hr, (int)respType, (int)respLen, (int)respLen, resp);

    return 0;
}