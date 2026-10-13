#!/usr/bin/env node
import { execSync } from 'node:child_process';

try {
  execSync('tsc -b .', { stdio: 'inherit' });
} catch (error) {
  process.exit(error.status ?? 1);
}
