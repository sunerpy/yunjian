---
license: apache-2.0
---
## Chinese-English ASR model using k2-zipformer-streaming
### AIShell-1 and Wenetspeech testset results with modified-beam-search streaming decode using epoch-14.pt
| decode_method | AIShell-1 | TEST_NET | TEST_MEETING |
|------------------|-----------|----------|--------------|
| modified_beam_search      | 3.04      | 8.97     | 8.83         ||
|       + SF （lm scale=0.1）      | 2.77      | -     | -         ||


### Training and decoding commands
```
nohup ./pruned_transducer_stateless7_streaming/train.py --world-size 8 --num-epochs 30 --start-epoch 1 --feedforward-dims "1024,1024,1536,1536,1024" --exp-dir pruned_transducer_stateless7_streaming/exp --max-duration 360 >  pruned_transducer_stateless7_streaming/exp/nohup.zipformer &
nohup ./pruned_transducer_stateless7_streaming/decode.py --epoch 14 --avg 1 --exp-dir ./pruned_transducer_stateless7_streaming/exp --max-duration 600 --decode-chunk-len 64 --decoding-method modified_beam_search --beam-size 4 > nohup.zipformer.deocode &
```

### Model unit is char+bpe as `data/lang_char_bpe/tokens.txt`

### Tips
some k2-fsa version and parameter is 
```
{'best_train_loss': inf, 'best_valid_loss': inf, 'best_train_epoch': -1, 'best_valid_epoch': -1, 'batch_idx_train': 0, 'lo
g_interval': 50, 'reset_interval': 200, 'valid_interval': 3000, 'feature_dim': 80, 'subsampling_factor': 4, 'warm_step': 2000, 'env_info': {'k2-version': '1.23.2', 'k2-build
-type': 'Release', 'k2-with-cuda': True, 'k2-git-sha1': 'a74f59dba1863cd9386ba4d8815850421260eee7', 'k2-git-date': 'Fri Dec 2 08:32:22 2022', 'lhotse-version': '1.5.0.dev+gi
t.8ce38fc.dirty', 'torch-version': '1.11.0+cu113', 'torch-cuda-available': True, 'torch-cuda-version': '11.3', 'python-version': '3.7', 'icefall-git-branch': 'master', 'icef
all-git-sha1': '11b08db-dirty', 'icefall-git-date': 'Thu Jan 12 10:19:21 2023', 'icefall-path': '/opt/conda/lib/python3.7/site-packages', 'k2-path': '/opt/conda/lib/python3.
7/site-packages/k2/__init__.py', 'lhotse-path': '/opt/conda/lib/python3.7/site-packages/lhotse/__init__.py', 'hostname': 'xxx', 'IP add
ress': 'x.x.x.x'}, 'world_size': 8, 'master_port': 12354, 'tensorboard': True, 'num_epochs': 30, 'start_epoch': 1, 'start_batch': 0, 'exp_dir': PosixPath('pruned_trans
ducer_stateless7_streaming/exp'), 'lang_dir': 'data/lang_char_bpe', 'base_lr': 0.01, 'lr_batches': 5000, 'lr_epochs': 3.5, 'context_size': 2, 'prune_range': 5, 'lm_scale': 0
.25, 'am_scale': 0.0, 'simple_loss_scale': 0.5, 'seed': 42, 'print_diagnostics': False, 'inf_check': False, 'save_every_n': 2000, 'keep_last_k': 30, 'average_period': 200, '
use_fp16': False, 'num_encoder_layers': '2,4,3,2,4', 'feedforward_dims': '1024,1024,1536,1536,1024', 'nhead': '8,8,8,8,8', 'encoder_dims': '384,384,384,384,384', 'attention_
dims': '192,192,192,192,192', 'encoder_unmasked_dims': '256,256,256,256,256', 'zipformer_downsampling_factors': '1,2,4,8,2', 'cnn_module_kernels': '31,31,31,31,31', 'decoder
_dim': 512, 'joiner_dim': 512, 'short_chunk_size': 50, 'num_left_chunks': 4, 'decode_chunk_len': 32, 'manifest_dir': PosixPath('data/fbank'), 'max_duration': 360, 'bucketing
_sampler': True, 'num_buckets': 300, 'concatenate_cuts': False, 'duration_factor': 1.0, 'gap': 1.0, 'on_the_fly_feats': False, 'shuffle': True, 'return_cuts': True, 'num_wor
kers': 8, 'enable_spec_aug': True, 'spec_aug_time_warp_factor': 80, 'enable_musan': True, 'training_subset': '12k_hour', 'blank_id': 0, 'vocab_size': 6254}
```