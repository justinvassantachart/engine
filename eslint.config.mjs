import { defineConfig } from 'eslint/config';
import tseslint from 'typescript-eslint';

export default defineConfig([
    ...tseslint.configs.recommended,
    {
        rules: {
            '@typescript-eslint/no-unused-vars': [
                'error',
                {
                    argsIgnorePattern: '^_+$',
                    varsIgnorePattern: '^_+$',
                    destructuredArrayIgnorePattern: '^_+$',
                },
            ],
        },
    }
]);
