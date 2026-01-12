import { defineConfig } from 'eslint/config';

export default defineConfig([
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
