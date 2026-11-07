/*
 * libticalcs2 oracle: runs the real TiLP library code (libticalcs2, model
 * CALC_TI84PCE_USB) against `haccerlink sim-serve` and records every raw
 * packet that crosses the "cable".
 *
 * libticalcs only touches the cable through ticables_cable_send() and
 * ticables_cable_recv(). We define both here and export them (-rdynamic), so
 * the dynamic linker binds libticalcs' calls to us instead of libticables:
 * the bytes go to the simulator over a pipe and into a transcript.
 *
 * usage: oracle <transcript-file> <haccerlink-binary> <op> [args...]
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include <ticables.h>
#include <tifiles.h>
#include <ticalcs.h>
#include <dusb_rpkt.h>

static FILE *transcript;
/* Optional: libticalcs' own dissection of every packet (ORACLE_DISSECT=file). */
static FILE *dissect_out;
static uint8_t dissect_first = 1;
static int to_sim = -1, from_sim = -1;

static uint8_t rbuf[2048];
static uint32_t rlen;

static void dump(const char *dir, const uint8_t *data, uint32_t len)
{
	fputs(dir, transcript);
	if (len == 0)
	{
		fputs(" ZLP", transcript);
	}
	for (uint32_t i = 0; i < len; i++)
	{
		fprintf(transcript, " %02X", data[i]);
	}
	fputc('\n', transcript);
	fflush(transcript);
	if (dissect_out && len > 0)
	{
		dusb_dissect(CALC_TI84PCE_USB, dissect_out, data, len, dir[0] == 'P' ? 0x02 : 0x01, &dissect_first);
		fflush(dissect_out);
	}
}

int ticables_cable_send(CableHandle *handle, uint8_t *data, uint32_t len)
{
	(void)handle;
	/* Each dusb_send() is one call; len == 0 is the CE ZLP workaround. */
	dump("PC>TI", data, len);
	uint32_t off = 0;
	while (off < len)
	{
		ssize_t n = write(to_sim, data + off, len - off);
		if (n <= 0)
		{
			return ERROR_WRITE_TIMEOUT;
		}
		off += n;
	}
	return 0;
}

int ticables_cable_recv(CableHandle *handle, uint8_t *data, uint32_t len)
{
	(void)handle;
	uint32_t off = 0;
	while (off < len)
	{
		ssize_t n = read(from_sim, data + off, len - off);
		if (n <= 0)
		{
			return ERROR_READ_TIMEOUT;
		}
		off += n;
	}
	/* dusb_recv() reads a 5-byte header, then the body: log whole packets. */
	if (rlen + len <= sizeof(rbuf))
	{
		memcpy(rbuf + rlen, data, len);
		rlen += len;
	}
	if (rlen >= 5)
	{
		uint32_t size = ((uint32_t)rbuf[0] << 24) | ((uint32_t)rbuf[1] << 16) | ((uint32_t)rbuf[2] << 8) | rbuf[3];
		if (rlen >= 5 + size)
		{
			dump("TI>PC", rbuf, rlen);
			rlen = 0;
		}
	}
	return 0;
}

static pid_t start_sim(const char *bin)
{
	int in[2], out[2];
	if (pipe(in) || pipe(out))
	{
		perror("pipe");
		exit(2);
	}
	pid_t pid = fork();
	if (pid == 0)
	{
		dup2(in[0], 0);
		dup2(out[1], 1);
		close(in[1]);
		close(out[0]);
		execl(bin, bin, "sim-serve", (char *)NULL);
		perror("exec haccerlink");
		_exit(127);
	}
	close(in[0]);
	close(out[1]);
	to_sim = in[1];
	from_sim = out[0];
	return pid;
}

static void fill_vr(VarRequest *vr, const char *name, const char *type)
{
	memset(vr, 0, sizeof(*vr));
	strncpy(vr->name, name, sizeof(vr->name) - 1);
	vr->type = (uint8_t)strtoul(type, NULL, 16);
}

#define NEED(n) do { if (argc < 4 + (n)) { fprintf(stderr, "%s: missing arguments\n", op); return 2; } } while (0)

int main(int argc, char **argv)
{
	if (argc < 4)
	{
		fprintf(stderr, "usage: %s <transcript> <haccerlink> <op> [args...]\n", argv[0]);
		return 2;
	}
	transcript = fopen(argv[1], "w");
	if (!transcript)
	{
		perror(argv[1]);
		return 2;
	}
	signal(SIGPIPE, SIG_IGN);
	if (getenv("ORACLE_DISSECT"))
	{
		dissect_out = fopen(getenv("ORACLE_DISSECT"), "w");
	}
	pid_t sim = start_sim(argv[2]);
	const char *op = argv[3];
	char **a = argv + 4;

	ticables_library_init();
	tifiles_library_init();
	ticalcs_library_init();

	CableHandle *cable = ticables_handle_new(CABLE_NUL, PORT_1);
	CalcHandle *calc = ticalcs_handle_new(CALC_TI84PCE_USB);
	if (!cable || !calc)
	{
		fprintf(stderr, "could not create handles\n");
		return 2;
	}
	int ret = ticalcs_cable_attach(calc, cable);
	if (ret)
	{
		fprintf(stderr, "attach failed: %d\n", ret);
		return 2;
	}

	/* TiLP checks readiness before doing anything. */
	ret = ticalcs_calc_isready(calc);
	if (!ret)
	{
		VarRequest vr, vr2;
		if (!strcmp(op, "ready"))
		{
		}
		else if (!strcmp(op, "info"))
		{
			CalcInfos infos;
			ret = ticalcs_calc_get_version(calc, &infos);
			if (!ret)
			{
				fprintf(stderr, "os=%s boot=%s id=%s\n", infos.os_version, infos.boot_version, infos.main_calc_id);
			}
		}
		else if (!strcmp(op, "memfree"))
		{
			uint32_t ram, flash;
			ret = ticalcs_calc_get_memfree(calc, &ram, &flash);
		}
		else if (!strcmp(op, "ls"))
		{
			GNode *vars = NULL, *apps = NULL;
			ret = ticalcs_calc_get_dirlist(calc, &vars, &apps);
		}
		else if (!strcmp(op, "send"))
		{
			NEED(1);
			ret = ticalcs_calc_send_var2(calc, MODE_NORMAL, a[0]);
		}
		else if (!strcmp(op, "sendapp"))
		{
			NEED(1);
			ret = ticalcs_calc_send_app2(calc, a[0]);
		}
		else if (!strcmp(op, "sendos"))
		{
			NEED(1);
			ret = ticalcs_calc_send_os2(calc, a[0]);
		}
		else if (!strcmp(op, "get"))
		{
			NEED(3);
			fill_vr(&vr, a[0], a[1]);
			ret = ticalcs_calc_recv_var2(calc, MODE_NORMAL, a[2], &vr);
		}
		else if (!strcmp(op, "getapp"))
		{
			NEED(2);
			fill_vr(&vr, a[0], "24");
			ret = ticalcs_calc_recv_app2(calc, a[1], &vr);
		}
		else if (!strcmp(op, "rm"))
		{
			NEED(2);
			fill_vr(&vr, a[0], a[1]);
			ret = ticalcs_calc_del_var(calc, &vr);
		}
		else if (!strcmp(op, "mv"))
		{
			NEED(3);
			fill_vr(&vr, a[0], a[1]);
			fill_vr(&vr2, a[2], a[1]);
			ret = ticalcs_calc_rename_var(calc, &vr, &vr2);
		}
		else if (!strcmp(op, "archive") || !strcmp(op, "unarchive"))
		{
			NEED(2);
			fill_vr(&vr, a[0], a[1]);
			ret = ticalcs_calc_change_attr(calc, &vr, !strcmp(op, "archive") ? ATTRB_ARCHIVED : ATTRB_NONE);
		}
		else if (!strcmp(op, "screenshot"))
		{
			CalcScreenCoord sc;
			uint8_t *bitmap = NULL;
			memset(&sc, 0, sizeof(sc));
			sc.format = SCREEN_FULL;
			ret = ticalcs_calc_recv_screen(calc, &sc, &bitmap);
		}
		else if (!strcmp(op, "key"))
		{
			NEED(1);
			ret = ticalcs_calc_send_key(calc, (uint32_t)strtoul(a[0], NULL, 16));
		}
		else if (!strcmp(op, "run"))
		{
			NEED(2);
			fill_vr(&vr, a[0], a[1]);
			ret = ticalcs_calc_execute(calc, &vr, "");
		}
		else if (!strcmp(op, "getclock"))
		{
			CalcClock c;
			ret = ticalcs_calc_get_clock(calc, &c);
			if (!ret)
			{
				fprintf(stderr, "%04u-%02u-%02u %02u:%02u:%02u\n", c.year, c.month, c.day, c.hours, c.minutes, c.seconds);
			}
		}
		else if (!strcmp(op, "setclock"))
		{
			NEED(8);
			CalcClock c;
			memset(&c, 0, sizeof(c));
			c.year = atoi(a[0]);
			c.month = atoi(a[1]);
			c.day = atoi(a[2]);
			c.hours = atoi(a[3]);
			c.minutes = atoi(a[4]);
			c.seconds = atoi(a[5]);
			c.date_format = atoi(a[6]);
			c.time_format = atoi(a[7]);
			c.state = 1;
			ret = ticalcs_calc_set_clock(calc, &c);
		}
		else
		{
			fprintf(stderr, "unknown op %s\n", op);
			ret = -1;
		}
	}

	if (ret)
	{
		char *msg = NULL;
		ticalcs_error_get(ret, &msg);
		fprintf(stderr, "libticalcs error %d: %s\n", ret, msg ? msg : "?");
	}

	close(to_sim);
	close(from_sim);
	waitpid(sim, NULL, 0);
	fclose(transcript);
	return ret ? 1 : 0;
}
