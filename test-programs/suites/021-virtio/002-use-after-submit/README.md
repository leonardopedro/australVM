Expected compile failure: `free_buffer(buffer)` after `submit(buffer, …)`
consumed the linear record. The pinned JSON diagnostic (linearity error on
the use-after-submit line) is in austral-stderr.txt.