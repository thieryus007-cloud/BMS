Samples: 74  of event 'cycles:P', 4000 Hz, Event count (approx.): 21685569 lost: 0/0 drop: 0/11
Overhead  Shared Object     Symbol
  19.00%  libc.so.6         [.] 0x000000000008ea90
  11.35%  [kernel]          [k] memset
   8.65%  daly-bms-server   [.] 0x00000000004d1724
   7.54%  [kernel]          [k] __mark_inode_dirty
   6.59%  libc.so.6         [.] 0x0000000000090d68
   4.38%  libc.so.6         [.] 0x000000000008fbf8
   4.09%  libc.so.6         [.] 0x0000000000092a04
   4.01%  daly-bms-server   [.] 0x000000000071ae70
   3.96%  [kernel]          [k] fpsimd_load_state
   3.03%  [kernel]          [k] build_open_flags
   2.81%  daly-bms-server   [.] 0x00000000004a9e68
   2.53%  daly-bms-server   [.] 0x00000000004aa000
   2.42%  daly-bms-server   [.] 0x00000000001fe6e8
   2.15%  daly-bms-server   [.] 0x00000000004d171c
   1.71%  [kernel]          [k] __sys_socket
   1.65%  daly-bms-server   [.] 0x00000000001594d4
   1.65%  libc.so.6         [.] 0x000000000009d158
   1.26%  libc.so.6         [.] 0x000000000008eadc
   1.24%  daly-bms-server   [.] 0x00000000000e3870
   0.95%  daly-bms-server   [.] 0x00000000002cea18
   0.85%  daly-bms-server   [.] 0x0000000000100f18
   0.85%  daly-bms-server   [.] 0x00000000004d1708
   0.74%  daly-bms-server   [.] 0x00000000001fd740
   0.70%  daly-bms-server   [.] 0x00000000000b311c
   0.65%  daly-bms-server   [.] 0x0000000000201de4
   0.62%  daly-bms-server   [.] 0x00000000002150b4
   0.56%  daly-bms-server   [.] 0x000000000051c690
   0.49%  daly-bms-server   [.] 0x00000000004ab5b0
   0.47%  [kernel]          [k] ktime_get_coarse_real_ts64
   0.43%  daly-bms-server   [.] 0x000000000030bab4
   0.37%  daly-bms-server   [.] 0x00000000000e9c78
   0.37%  daly-bms-server   [.] 0x00000000000e0d54
   0.34%  daly-bms-server   [.] 0x0000000000159a00
   0.32%  libc.so.6         [.] xdr_long
   0.32%  libc.so.6         [.] 0x0000000000091548
   0.28%  daly-bms-server   [.] 0x00000000001dc70c
   0.22%  daly-bms-server   [.] 0x00000000000df9ac
   0.21%  [kernel]          [k] __ip4_datagram_connect
   0.18%  libc.so.6         [.] 0x000000000010d184
   0.04%  libc.so.6         [.] 0x000000000008e964
   0.00%  daly-bms-server   [.] 0x00000000004ab30c
   0.00%  [overlay]         [k] ovl_inode_version_get
   0.00%  [kernel]          [k] override_creds
   0.00%  [kernel]          [k] futex_wake
   0.00%  daly-bms-server   [.] 0x00000000004ac110
Too slow to read ring buffer (change period (-c/-F) or limit CPUs (-C)

amples: 131  of event 'cycles:P', Event count (approx.): 37223230
  Children      Self  Command          Shared Object      Symbol
+  100.00%     0.00%  tokio-rt-worker  libc.so.6          [.] 0x00007fff0bffbf1c
+  100.00%     0.00%  tokio-rt-worker  libc.so.6          [.] 0x00007fff0bf92030
+  100.00%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bab97ec8
+  100.00%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bab9fc60
+   97.45%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bab9dee8
+   53.99%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baba8548
+   34.91%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba88ee14
+   32.88%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555babaa4c8
+   20.73%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555babaa508
+   19.81%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] el0t_64_sync
+   19.81%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] el0t_64_sync_handler
+   19.81%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] el0_svc
+   19.77%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] do_el0_svc
+   19.77%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] el0_svc_common.constprop.0
+   19.77%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] invoke_syscall
+   19.18%     6.65%  tokio-rt-worker  libc.so.6          [.] malloc
+   17.94%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baba8c78
+   15.80%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baba9e60
+   15.80%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555babab5b0
+   13.61%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bababd20
+   13.61%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555babac274
+   13.55%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba891f78
+   13.26%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bab9b04c
+   13.26%     0.02%  tokio-rt-worker  libc.so.6          [.] getaddrinfo
+   13.25%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555bab9b3e4
+   13.02%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba800cb0
+   12.90%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba99603c
+   11.93%     4.22%  tokio-rt-worker  libc.so.6          [.] epoll_pwait
+   11.90%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba7e1038
+    9.31%     9.00%  tokio-rt-worker  libc.so.6          [.] cfree
+    8.35%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba7dd988
+    7.92%     0.00%  tokio-rt-worker  libc.so.6          [.] _nss_files_gethostbyname4_r
+    7.92%     0.00%  tokio-rt-worker  libc.so.6          [.] __nss_files_fopen
+    7.71%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] __arm64_sys_epoll_pwait
+    7.71%     0.00%  tokio-rt-worker  [kernel.kallsyms]  [k] do_epoll_pwait.part.0
+    7.70%     5.62%  tokio-rt-worker  [kernel.kallsyms]  [k] do_epoll_wait
+    7.23%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa11dc8
+    7.23%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa0ca80
+    6.84%     0.00%  tokio-rt-worker  libc.so.6          [.] realloc
+    6.62%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555ba7e4318
+    5.79%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa0ec78
+    5.79%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa15c50

+      19.51%  daly-bms-server   [.] 0x00000000004aa4c4
   9.68%  [overlay]         [k] ovl_permission
   7.66%  daly-bms-server   [.] 0x000000000031f280
   4.99%  libc.so.6         [.] 0x0000000000092a04
   4.97%  [kernel]          [k] sk_destruct
   4.45%  libc.so.6         [.] 0x000000000008ea90
   3.35%  daly-bms-server   [.] 0x0000000000348b30
   3.30%  daly-bms-server   [.] 0x00000000001005dc
   3.30%  libc.so.6         [.] mtx_timedlock
   3.30%  libc.so.6         [.] xdr_array
   3.16%  libc.so.6         [.] mq_open
   2.89%  daly-bms-server   [.] 0x00000000000dfec0
   2.89%  daly-bms-server   [.] 0x00000000001e46f0
   2.60%  daly-bms-server   [.] 0x000000000071abd0
   2.31%  [kernel]          [k] fdget_pos
   2.21%  daly-bms-server   [.] 0x00000000000ab5ac
   1.97%  libc.so.6         [.] 0x00000000000911dc
   1.76%  [kernel]          [k] _raw_spin_unlock_irq
   1.76%  daly-bms-server   [.] 0x000000000030ea68
   1.68%  daly-bms-server   [.] 0x000000000049bda0
   1.68%  libc.so.6         [.] 0x0000000000090d68
   1.67%  libc.so.6         [.] 0x0000000000090ce0
   1.59%  daly-bms-server   [.] 0x00000000004aa080
   1.49%  daly-bms-server   [.] 0x00000000002bc45c           
   1.39%  [kernel]          [k] __arm64_sys_epoll_pwait
   1.39%  libc.so.6         [.] _IO_file_xsputn
   1.36%  libc.so.6         [.] 0x000000000008eadc
   1.12%  [kernel]          [k] kmem_cache_alloc_noprof
   0.49%  libc.so.6         [.] 0x000000000007b964
   0.03%  daly-bms-server   [.] 0x000000000007e990
   0.02%  libc.so.6         [.] 0x000000000010b694
+    5.79%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa26fe4
+    5.79%     0.00%  tokio-rt-worker  daly-bms-server    [.] 0x00005555baa903f8
+    5.45%     5.45%  tokio-rt-worker  libc.so.6          [.] 0x000000000009d20c
Cannot load tips.txt file, please install perf!
