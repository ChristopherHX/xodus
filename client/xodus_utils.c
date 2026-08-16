/*
 * Copyright 2026 Paweł Lidwin
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin St, Fifth Floor, Boston, MA 02110-1301, USA
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include "xodus_utils.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

#define TRACE(...) fprintf(stderr, __VA_ARGS__)

static BOOL pipe_write_full( HANDLE h, const void *buf, DWORD len )
{
    const BYTE *ptr = buf;
    DWORD written;

    while (len)
    {
        fprintf(stderr, "remaining %d\n", (int)len);
        if (!WriteFile( h, ptr, len, &written, NULL ) || !written) return FALSE;
        ptr += written;
        len -= written;
    }
    fprintf(stderr, "remaining %d\n", (int)len);
    return TRUE;
}

static BOOL pipe_read_full( HANDLE h, void *buf, DWORD len )
{
    BYTE *ptr = buf;
    DWORD got;

    while (len)
    {
        fprintf(stderr, "remaining read %d\n", (int)len);
        if (!ReadFile( h, ptr, len, &got, NULL ) || !got) return FALSE;
        ptr += got;
        len -= got;
    }
    fprintf(stderr, "remaining read %d\n", (int)len);
    return TRUE;
}

HRESULT shim_connect( struct shim_channel *shim )
{
    InitializeCriticalSection( &shim->lock );
    shim->toShim = GetStdHandle( STD_OUTPUT_HANDLE );
    shim->fromShim = GetStdHandle( STD_INPUT_HANDLE );
    shim->pid = 0;
    shim->process = NULL;
    shim->active = TRUE;
    return S_OK;
}

HRESULT shim_start( struct shim_channel *shim, const WCHAR *envVar )
{
    return E_FAIL;
}

void shim_stop( struct shim_channel *shim )
{
    DeleteCriticalSection( &shim->lock );
    if (!shim->active) return;

    TRACE( "shim %p.\n", shim );

    CloseHandle( shim->toShim );
    GenerateConsoleCtrlEvent( CTRL_BREAK_EVENT, shim->pid );
    if (WaitForSingleObject( shim->process, 1000 ) == WAIT_TIMEOUT)
        TerminateProcess( shim->process, 1 );
    CloseHandle( shim->fromShim );
    CloseHandle( shim->process );
    shim->active = FALSE;
}

HRESULT shim_call( struct shim_channel *shim, UINT16 type, const char *payload, UINT16 payloadLen,
                    UINT16 *respType, char **resp, UINT16 *respLen )
{
    struct shim_header hdr = { XML_MAGIC, type, payloadLen };
    struct shim_header rhdr;
    HRESULT hr = S_OK;

    TRACE( "shim %p, type %u, payload %p, payloadLen %u.\n", shim, type, payload, payloadLen );

    EnterCriticalSection( &shim->lock );

    if (!shim->active)
    {
        hr = E_ABORT;
        goto done;
    }

    if (!pipe_write_full( shim->toShim, &hdr, sizeof(hdr) ) ||
        (payloadLen && !pipe_write_full( shim->toShim, payload, payloadLen )))
    {
        shim->active = FALSE;
        hr = HRESULT_FROM_WIN32( GetLastError() );
        goto done;
    }

    if (!pipe_read_full( shim->fromShim, &rhdr, sizeof(rhdr) ) || rhdr.magic != XML_MAGIC)
    {
        shim->active = FALSE;
        hr = E_FAIL;
        goto done;
    }

    if (!(*resp = calloc( 1, (SIZE_T)rhdr.length + 1 )))
    {
        hr = E_OUTOFMEMORY;
        goto done;
    }
    if (rhdr.length && !pipe_read_full( shim->fromShim, *resp, rhdr.length ))
    {
        free( *resp );
        *resp = NULL;
        shim->active = FALSE;
        hr = E_FAIL;
        goto done;
    }
    *respType = rhdr.type;
    *respLen = rhdr.length;

done:
    LeaveCriticalSection( &shim->lock );
    return hr;
}
