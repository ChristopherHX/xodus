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

#ifndef __WINE_XGAMERUNTIME_XODUS_UTILS_H
#define __WINE_XGAMERUNTIME_XODUS_UTILS_H

struct shim_header
{
    UINT32 magic;
    UINT16 type;
    UINT16 length;
};

#define XML_MAGIC 0x58445358
#define PING_REQUEST 1
#define PING_RESPONSE 2
#define MSA_TOKEN_REQUEST  3
#define MSA_TOKEN_RESPONSE  4

struct shim_channel
{
    HANDLE process;
    DWORD pid;
    HANDLE toShim;    /* parent's write end -> child's stdin  */
    HANDLE fromShim;  /* parent's read end  <- child's stdout */
    CRITICAL_SECTION lock;
    BOOL active;
};

HRESULT shim_start( struct shim_channel *shim, const WCHAR *envVar );
HRESULT shim_connect( struct shim_channel *shim );
void shim_stop( struct shim_channel *shim );
HRESULT shim_call( struct shim_channel *shim, UINT16 type, const char *payload, UINT16 payloadLen,
                    UINT16 *respType, char **resp, UINT16 *respLen );

#endif
